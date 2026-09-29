use calckernel::{
    EmitWasmOptions, KirBoundsMode, KirBuildConfig, KirOptimizationLevel, KirOverflowMode,
    KirSanitizerMode, KirTargetProfile, KirVerifiedProgramState, KirWasmFeatures, SourceFile,
    VectorPredicate, build_kir_module_with_profile, check, check_vectorization_trial_independently,
    discover_vectorization_candidates, emit_wasm_kir_module, import_contract_facts, lower_to_mir,
    prepare_vectorization_trial, run_kir_pass_pipeline, validate_kir_module,
};

const POLYNOMIAL: &str = r#"
export unsafe fn polynomial(input: slice<f64>, output: slice<f64>, n: u32) -> void
contract {
  requires n <= input.len && n <= output.len;
  requires noalias(input, output);
  effects read(input), write(output);
} {
  let i: u32 = 0;
  while i < n {
    let x: f64 = input[i];
    let value0: f64 = 1.0 / 16.0;
    let value1: f64 = value0 * x - 1.0 / 8.0;
    let value2: f64 = value1 * x + 1.0 / 4.0;
    let value3: f64 = value2 * x - 1.0 / 2.0;
    let value4: f64 = value3 * x + 1.0 / 2.0;
    let value5: f64 = value4 * x - 1.0 / 4.0;
    let value6: f64 = value5 * x + 1.0 / 8.0;
    output[i] = value6 * x - 1.0 / 16.0;
    i = i + 1;
  }
}
"#;

const POLYNOMIAL_SLICE_LENGTH: &str = r#"
export unsafe fn ck_bench_polynomial(input: slice<f64>, out: slice<f64>) -> void contract {
  requires input.len == out.len;
  requires noalias(input, out);
  effects read(input), write(out);
} {
  let i: u32 = 0;
  while i < input.len {
    let x: f64 = input[i];
    let p0: f64 = 0.0625;
    let p1: f64 = p0 * x - 0.125;
    let p2: f64 = p1 * x + 0.25;
    let p3: f64 = p2 * x - 0.5;
    let p4: f64 = p3 * x + 0.5;
    let p5: f64 = p4 * x - 0.25;
    let p6: f64 = p5 * x + 0.125;
    out[i] = p6 * x - 0.0625;
    i = i + 1;
  }
}
"#;

fn polynomial_state(source: &str, level: KirOptimizationLevel) -> KirVerifiedProgramState {
    let checked = check(&SourceFile::new("polynomial-uf.ck", source));
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
    let contracts =
        import_contract_facts(&module, &checked.checked_program, 0).expect("contract facts");
    let optimized = run_kir_pass_pipeline(module, level, Some(&contracts));
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

#[test]
fn polynomial_direct_map_should_offer_checked_vf2_uf4_candidate() {
    let profile = KirTargetProfile::webassembly_with_features(KirWasmFeatures::Simd128);
    assert_eq!(profile.maximum_interleave_factor(), 4);
    assert_eq!(
        KirTargetProfile::webassembly_with_features(KirWasmFeatures::Baseline)
            .maximum_interleave_factor(),
        1
    );

    let state = polynomial_state(POLYNOMIAL, KirOptimizationLevel::O2);
    let candidate = discover_vectorization_candidates(&state)
        .candidates
        .into_iter()
        .find(|candidate| {
            candidate.wasm_affine.is_some()
                && candidate.vf == 2
                && candidate.uf == 4
                && candidate.reduction.is_none()
                && candidate.diamond.is_none()
        })
        .expect("strict direct polynomial map should admit VF2/UF4");
    assert!(
        candidate
            .operations
            .iter()
            .all(|operation| operation.semantics == calckernel::KirCostSemantics::StrictFloat)
    );

    let prepared = prepare_vectorization_trial(&state, &candidate).expect("UF4 materialization");
    assert_eq!(prepared.plan.vf, 2);
    assert_eq!(prepared.plan.uf, 4);
    assert_eq!(
        prepared
            .plan
            .predicates
            .iter()
            .filter(|predicate| matches!(predicate, VectorPredicate::WasmSliceRange { .. }))
            .count(),
        2,
        "both complete physical slice ranges must remain guarded"
    );
    assert_eq!(
        check_vectorization_trial_independently(
            &state,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge,
        ),
        Ok(()),
        "source-only checker must accept the interleaved direct map"
    );
}

#[test]
fn polynomial_o3_should_choose_uf4_using_existing_scope_cost() {
    let state = polynomial_state(POLYNOMIAL_SLICE_LENGTH, KirOptimizationLevel::O3);
    let function = state
        .module()
        .functions
        .iter()
        .find(|function| function.name == "ck_bench_polynomial")
        .expect("polynomial function");
    let loads = function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .filter(|instruction| {
            matches!(
                instruction.kind,
                calckernel::KirInstructionKind::VectorLoad { .. }
            )
        })
        .count();
    let stores = function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .filter(|instruction| {
            matches!(
                instruction.kind,
                calckernel::KirInstructionKind::VectorStore { .. }
            )
        })
        .count();
    assert_eq!(
        loads, 4,
        "O3 should select one contiguous load per UF=4 group"
    );
    assert_eq!(
        stores, 4,
        "O3 should select one contiguous store per UF=4 group"
    );
}

#[test]
fn polynomial_slice_length_bound_should_reuse_entry_length_for_uf1_uf2_and_uf4() {
    let state = polynomial_state(POLYNOMIAL_SLICE_LENGTH, KirOptimizationLevel::O2);
    let candidates = discover_vectorization_candidates(&state)
        .candidates
        .into_iter()
        .filter(|candidate| candidate.wasm_affine.is_some() && candidate.vf == 2)
        .collect::<Vec<_>>();
    assert_eq!(
        candidates
            .iter()
            .map(|candidate| candidate.uf)
            .collect::<Vec<_>>(),
        [1, 2, 4],
        "source-length direct maps should offer the same checked UF variants"
    );
    for candidate in candidates {
        let prepared = prepare_vectorization_trial(&state, &candidate)
            .unwrap_or_else(|error| panic!("UF{} materialization failed: {error}", candidate.uf));
        let checked = check_vectorization_trial_independently(
            &state,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge,
        );
        assert_eq!(checked, Ok(()), "UF{} source-length trial", candidate.uf);
    }
}

#[test]
fn polynomial_direct_map_checker_should_reject_forged_unroll_partition() {
    let state = polynomial_state(POLYNOMIAL, KirOptimizationLevel::O2);
    let candidate = discover_vectorization_candidates(&state)
        .candidates
        .into_iter()
        .find(|candidate| candidate.wasm_affine.is_some() && candidate.vf == 2 && candidate.uf == 4)
        .expect("VF2/UF4 candidate");
    let mut prepared = prepare_vectorization_trial(&state, &candidate).expect("UF4 trial");
    let second = prepared
        .plan
        .memory_groups
        .iter_mut()
        .find(|group| group.unroll_index == 1)
        .expect("second interleave memory group");
    second.unroll_index = 0;
    assert!(
        check_vectorization_trial_independently(
            &state,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge,
        )
        .is_err()
    );
}

#[test]
fn polynomial_direct_map_checker_should_reject_forged_offsets_recurrence_and_memoryssa() {
    let state = polynomial_state(POLYNOMIAL, KirOptimizationLevel::O2);
    let candidate = discover_vectorization_candidates(&state)
        .candidates
        .into_iter()
        .find(|candidate| candidate.wasm_affine.is_some() && candidate.vf == 2 && candidate.uf == 4)
        .expect("VF2/UF4 candidate");
    let prepared = prepare_vectorization_trial(&state, &candidate).expect("UF4 trial");
    assert!(
        check_vectorization_trial_independently(
            &state,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge,
        )
        .is_ok()
    );

    let mut wrong_offset = prepared.trial.clone();
    let function = wrong_offset
        .module_mut()
        .functions
        .iter_mut()
        .find(|function| function.id == candidate.function)
        .unwrap();
    let original_header = state
        .module()
        .functions
        .iter()
        .find(|function| function.id == candidate.function)
        .unwrap()
        .blocks
        .iter()
        .find(|block| block.id == candidate.header)
        .unwrap();
    let induction_index = original_header
        .params
        .iter()
        .position(|param| param.value == candidate.induction)
        .unwrap();
    let first_chunk = function
        .blocks
        .iter()
        .find(|block| block.label == "loop_simd_header")
        .unwrap()
        .params[induction_index]
        .value;
    let fourth_load = prepared
        .plan
        .memory_groups
        .iter()
        .find(|group| {
            group.unroll_index == 3 && group.access == calckernel::VectorMemoryAccessKind::Read
        })
        .unwrap()
        .vector_instruction;
    let instruction = function
        .blocks
        .iter_mut()
        .flat_map(|block| &mut block.instructions)
        .find(|instruction| instruction.id == fourth_load)
        .unwrap();
    let calckernel::KirInstructionKind::VectorLoad { access, .. } = &mut instruction.kind else {
        panic!("fourth interleave load");
    };
    access.start = first_chunk;
    assert!(
        check_vectorization_trial_independently(
            &state,
            &wrong_offset,
            &prepared.plan,
            &prepared.charge,
        )
        .is_err(),
        "checker accepted the first chunk address for the fourth group"
    );

    let mut wrong_recurrence = prepared.trial.clone();
    let function = wrong_recurrence
        .module_mut()
        .functions
        .iter_mut()
        .find(|function| function.id == candidate.function)
        .unwrap();
    let vector_induction = function
        .blocks
        .iter()
        .find(|block| block.label == "loop_simd_header")
        .unwrap()
        .params[induction_index]
        .value;
    let vector_body = function
        .blocks
        .iter_mut()
        .find(|block| block.label == "loop_simd_body")
        .unwrap();
    let calckernel::KirTerminator::Jump { edge } = &mut vector_body.terminator else {
        panic!("vector backedge");
    };
    edge.args[induction_index] = vector_induction;
    assert!(
        check_vectorization_trial_independently(
            &state,
            &wrong_recurrence,
            &prepared.plan,
            &prepared.charge,
        )
        .is_err(),
        "checker accepted a non-advancing vector backedge"
    );

    let mut wrong_memory = prepared.trial.clone();
    let function = wrong_memory
        .module_mut()
        .functions
        .iter_mut()
        .find(|function| function.id == candidate.function)
        .unwrap();
    let fourth_store = prepared
        .plan
        .memory_groups
        .iter()
        .find(|group| {
            group.unroll_index == 3 && group.access == calckernel::VectorMemoryAccessKind::Write
        })
        .unwrap();
    let stale = function
        .blocks
        .iter()
        .find(|block| block.label == "loop_simd_header")
        .unwrap()
        .memory_params
        .iter()
        .find(|param| param.region == fourth_store.region)
        .unwrap()
        .version;
    let body = function
        .blocks
        .iter_mut()
        .find(|block| block.label == "loop_simd_body")
        .unwrap();
    let instruction = body
        .instructions
        .iter_mut()
        .find(|instruction| instruction.id == fourth_store.vector_instruction)
        .unwrap();
    let memory = instruction.memory.as_mut().expect("fourth store MemorySSA");
    memory.input = stale;
    assert!(
        check_vectorization_trial_independently(
            &state,
            &wrong_memory,
            &prepared.plan,
            &prepared.charge,
        )
        .is_err(),
        "checker accepted a skipped interleave store in MemorySSA"
    );
}

#[test]
fn polynomial_uf4_checker_rejects_valid_kir_wrong_scalar_tail_edge() {
    let state = polynomial_state(POLYNOMIAL, KirOptimizationLevel::O2);
    let candidate = discover_vectorization_candidates(&state)
        .candidates
        .into_iter()
        .find(|candidate| candidate.wasm_affine.is_some() && candidate.vf == 2 && candidate.uf == 4)
        .expect("VF2/UF4 candidate");
    let prepared = prepare_vectorization_trial(&state, &candidate).expect("UF4 trial");
    let mut wrong_tail = prepared.trial.clone();
    let function = wrong_tail
        .module_mut()
        .functions
        .iter_mut()
        .find(|function| function.id == candidate.function)
        .unwrap();
    let original_header = state
        .module()
        .functions
        .iter()
        .find(|function| function.id == candidate.function)
        .unwrap()
        .blocks
        .iter()
        .find(|block| block.id == candidate.header)
        .unwrap();
    let induction_index = original_header
        .params
        .iter()
        .position(|param| param.value == candidate.induction)
        .unwrap();
    let header = function
        .blocks
        .iter_mut()
        .find(|block| block.label == "loop_simd_header")
        .unwrap();
    let calckernel::KirTerminator::Branch { else_edge, .. } = &mut header.terminator else {
        panic!("vector header branch");
    };
    else_edge.args[induction_index] = candidate.bound;
    assert!(validate_kir_module(wrong_tail.module()).errors.is_empty());
    assert!(
        check_vectorization_trial_independently(
            &state,
            &wrong_tail,
            &prepared.plan,
            &prepared.charge,
        )
        .is_err(),
        "checker accepted a valid-KIR scalar tail that skips the final element"
    );
}

#[test]
fn polynomial_uf4_checker_rejects_valid_kir_unused_preheader_division() {
    let state = polynomial_state(POLYNOMIAL, KirOptimizationLevel::O2);
    let candidate = discover_vectorization_candidates(&state)
        .candidates
        .into_iter()
        .find(|candidate| candidate.wasm_affine.is_some() && candidate.vf == 2 && candidate.uf == 4)
        .expect("VF2/UF4 candidate");
    let prepared = prepare_vectorization_trial(&state, &candidate).expect("UF4 trial");
    let mut trapping_preheader = prepared.trial.clone();
    let function = trapping_preheader
        .module_mut()
        .functions
        .iter_mut()
        .find(|function| function.id == candidate.function)
        .unwrap();
    let preheader = function
        .blocks
        .iter_mut()
        .find(|block| block.id == candidate.preheader)
        .unwrap();
    let added_constant = preheader
        .instructions
        .iter_mut()
        .rev()
        .find(|instruction| {
            matches!(&instruction.kind,
            calckernel::KirInstructionKind::ConstInt { value }
                if value == &candidate.minimum_trip.to_string())
        })
        .expect("minimum trip setup");
    added_constant.kind = calckernel::KirInstructionKind::Binary {
        op: calckernel::MirBinaryOp::Div,
        left: candidate.bound,
        right: candidate.bound,
        semantics: calckernel::KirArithmeticSemantics::Modular,
    };
    assert!(
        validate_kir_module(trapping_preheader.module())
            .errors
            .is_empty()
    );
    assert!(
        check_vectorization_trial_independently(
            &state,
            &trapping_preheader,
            &prepared.plan,
            &prepared.charge,
        )
        .is_err(),
        "checker accepted an unused preheader division that traps at n=0"
    );
}

#[test]
fn polynomial_source_header_division_must_block_all_vector_candidates_and_trials() {
    let state = polynomial_state(POLYNOMIAL, KirOptimizationLevel::O2);
    let candidate = discover_vectorization_candidates(&state)
        .candidates
        .into_iter()
        .find(|candidate| candidate.wasm_affine.is_some() && candidate.vf == 2 && candidate.uf == 4)
        .expect("VF2/UF4 candidate");
    let mut changed = state.module().clone();
    let ids = state.ids();
    let function = changed
        .functions
        .iter_mut()
        .find(|function| function.id == candidate.function)
        .unwrap();
    let bound = function
        .params
        .iter()
        .find(|param| param.name == "n")
        .unwrap()
        .value;
    let zero = function
        .blocks
        .iter()
        .find(|block| block.id == candidate.preheader)
        .unwrap()
        .instructions
        .iter()
        .find_map(|instruction| match &instruction.kind {
            calckernel::KirInstructionKind::ConstInt { value } if value == "0" => {
                Some(instruction.results[0].value)
            }
            _ => None,
        })
        .unwrap();
    let header = function
        .blocks
        .iter_mut()
        .find(|block| block.id == candidate.header)
        .unwrap();
    header.instructions.insert(
        0,
        calckernel::KirInstruction {
            id: calckernel::InstructionId::from_index(ids.next_instruction),
            results: vec![calckernel::KirResult {
                value: calckernel::ValueId::from_index(ids.next_value),
                type_node: calckernel::MirType::Primitive(calckernel::MirPrimitiveTypeName::U32)
                    .into(),
            }],
            kind: calckernel::KirInstructionKind::Binary {
                op: calckernel::MirBinaryOp::Div,
                left: bound,
                right: zero,
                semantics: calckernel::KirArithmeticSemantics::Modular,
            },
            memory: None,
            effect: None,
        },
    );
    let changed = KirVerifiedProgramState::from_parts(
        changed,
        state.contract_facts().cloned(),
        state.proofs().clone(),
        state.eliminated_guards().to_vec(),
        0,
    )
    .expect("valid mutated pre-state");
    assert!(validate_kir_module(changed.module()).errors.is_empty());
    let discovery = discover_vectorization_candidates(&changed);
    assert!(
        discovery
            .candidates
            .iter()
            .all(|trial| trial.header != candidate.header),
        "proposer offered a loop that skips a trapping source header"
    );
    let prepared = prepare_vectorization_trial(&changed, &candidate).expect("adversarial trial");
    assert!(
        check_vectorization_trial_independently(
            &changed,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge,
        )
        .is_err(),
        "checker accepted a trial that writes before the source-header trap"
    );
}

#[test]
fn polynomial_direct_map_without_source_noalias_must_not_use_unguarded_simd() {
    let without_noalias = POLYNOMIAL.replace("  requires noalias(input, output);\n", "");
    let state = polynomial_state(&without_noalias, KirOptimizationLevel::O2);
    let discovery = discover_vectorization_candidates(&state);
    assert!(
        discovery.candidates.iter().all(|candidate| {
            candidate.uf == 1
                && (candidate.wasm_affine.is_none() || candidate.version_predicate.is_some())
        }),
        "missing noalias fact must not authorize an unguarded UF candidate: {discovery:#?}"
    );
}

#[test]
fn polynomial_direct_map_should_preserve_strict_bits_tails_and_trap_prefix() {
    if !crate::support::command::node_available() {
        return;
    }
    let state = polynomial_state(POLYNOMIAL, KirOptimizationLevel::O2);
    let candidate = discover_vectorization_candidates(&state)
        .candidates
        .into_iter()
        .find(|candidate| candidate.wasm_affine.is_some() && candidate.vf == 2 && candidate.uf == 4)
        .expect("VF2/UF4 candidate");
    let prepared = prepare_vectorization_trial(&state, &candidate).expect("UF4 trial");
    let options = EmitWasmOptions { opt_level: 3 };
    let binaries = [state.module(), prepared.trial.module()]
        .map(|module| emit_wasm_kir_module(module, options).expect("WASM binary"));
    let script = r#"
      const assert = require('node:assert/strict');
      const binaries = JSON.parse(process.argv[1]);
      const patterns = [
        0x0000000000000000n, 0x8000000000000000n,
        0x0000000000000001n, 0x8000000000000001n,
        0x3ff0000000000000n, 0xbff0000000000000n,
        0x7ff0000000000000n, 0xfff0000000000000n,
        0x7ff8000000000042n, 0xfff8000000000017n,
        0x3fd5555555555555n, 0xc004000000000000n
      ];
      function run(binary, n, inputCapacity, outputCapacity) {
        const { memory, polynomial } = new WebAssembly.Instance(
          new WebAssembly.Module(Uint8Array.from(binary))).exports;
        const raw = new Uint8Array(memory.buffer);
        raw.fill(0xa5);
        const view = new DataView(memory.buffer);
        const input = inputCapacity < n ? raw.length - inputCapacity * 8 : 1024;
        const output = outputCapacity < n ? raw.length - outputCapacity * 8 : 8192;
        for (let i = 0; i < inputCapacity; i++) {
          view.setBigUint64(input + i * 8, patterns[i % patterns.length], true);
        }
        for (let i = 0; i < outputCapacity; i++) {
          view.setBigUint64(output + i * 8, patterns[(i + 5) % patterns.length], true);
        }
        let trapped = false;
        try { polynomial(input, n, output, n, n); }
        catch (error) {
          assert(error instanceof WebAssembly.RuntimeError);
          trapped = true;
        }
        return { trapped, bytes: Buffer.from(raw) };
      }
      for (let n = 0; n <= 17; n++) {
        const scalar = run(binaries[0], n, n, n);
        const vector = run(binaries[1], n, n, n);
        assert.equal(vector.trapped, scalar.trapped, `trap mismatch at n=${n}`);
        assert(vector.bytes.equals(scalar.bytes), `strict output or tail mismatch at n=${n}`);
      }
      for (const [n,inputCapacity,outputCapacity] of [[9,5,9],[17,7,17],[17,17,9]]) {
        const scalar = run(binaries[0], n, inputCapacity, outputCapacity);
        const vector = run(binaries[1], n, inputCapacity, outputCapacity);
        assert.equal(scalar.trapped, true, `scalar fixture did not trap: ${n}/${inputCapacity}/${outputCapacity}`);
        assert.equal(vector.trapped, scalar.trapped, `trap mismatch: ${n}/${inputCapacity}/${outputCapacity}`);
        assert(vector.bytes.equals(scalar.bytes), `trap prefix differs: ${n}/${inputCapacity}/${outputCapacity}`);
      }
    "#;
    let output = std::process::Command::new("node")
        .args(["-e", script, &serde_json::to_string(&binaries).unwrap()])
        .output()
        .expect("Node strict polynomial oracle");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
