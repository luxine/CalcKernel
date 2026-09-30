use calckernel::*;

const NORMALIZATION: &str = r#"
export unsafe fn normalize(input: slice<f64>, out: slice<f64>) -> void contract {
  requires input.len != 0 && input.len == out.len;
  requires noalias(input, out);
  effects read(input), write(out);
} {
  let minimum: f64 = input[0];
  let maximum: f64 = input[0];
  let i: u32 = 1;
  while i < input.len {
    if input[i] < minimum { minimum = input[i]; }
    if input[i] > maximum { maximum = input[i]; }
    i = i + 1;
  }
  let range: f64 = maximum - minimum;
  let j: u32 = 0;
  while j < input.len {
    if range == 0.0 {
      out[j] = 0.0;
    } else {
      out[j] = (input[j] - minimum) / range;
    }
    j = j + 1;
  }
}
"#;

fn wasm_state(source: &str, level: KirOptimizationLevel) -> KirVerifiedProgramState {
    let checked = check(&SourceFile::new("normalization-unswitch.ck", source));
    assert_eq!(checked.diagnostics, [], "invalid test fixture:\n{source}");
    let mir = lower_to_mir(&checked.checked_program).expect("MIR lowering");
    let module = build_kir_module_with_profile(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::WebAssembly,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
        KirTargetProfile::webassembly_with_features(KirWasmFeatures::Simd128),
    )
    .expect("WASM KIR");
    let contracts =
        import_contract_facts(&module, &checked.checked_program, 0).expect("source contract facts");
    let optimized = run_kir_pass_pipeline(module, level, Some(&contracts));
    assert!(optimized.errors.is_empty(), "{:?}", optimized.errors);
    KirVerifiedProgramState::from_parts(
        optimized.artifact.expect("optimized KIR"),
        optimized.contract_facts,
        optimized.proofs,
        optimized.eliminated_guards,
        0,
    )
    .expect("verified KIR state")
}

#[test]
fn normalization_unswitch_should_leave_o0_scalar_state_unchanged() {
    let state = wasm_state(NORMALIZATION, KirOptimizationLevel::O0);
    let digest = state.kir_digest();
    assert!(discover_normalization_unswitch_candidates(&state).is_empty());
    assert_eq!(state.kir_digest(), digest);
    assert!(
        analyze_canonical_loops(&state.module().functions[0])
            .loops
            .iter()
            .any(|loop_info| loop_info.blocks.len() >= 5)
    );
}

#[test]
fn normalization_unswitch_should_clone_only_the_strict_false_arm_into_a_two_block_loop() {
    let before = wasm_state(NORMALIZATION, KirOptimizationLevel::O2);
    let candidates = discover_normalization_unswitch_candidates(&before);
    assert_eq!(candidates.len(), 1, "{}", print_kir_module(before.module()));
    let prepared = prepare_normalization_unswitch_trial(&before, &candidates[0])
        .expect("narrow normalization unswitch");
    check_normalization_unswitch_independently(
        &before,
        &prepared.trial,
        &prepared.plan,
        &prepared.charge,
    )
    .expect("independent transaction check");

    let function = prepared
        .trial
        .module()
        .functions
        .iter()
        .find(|function| function.id == candidates[0].function)
        .unwrap();
    let loops = analyze_canonical_loops(function).loops;
    let fast = loops
        .iter()
        .find(|loop_info| loop_info.header == prepared.plan.fast_header)
        .expect("new false-path loop is canonical");
    assert_eq!(fast.blocks.len(), 2, "{fast:#?}");
    assert_eq!(fast.exits.len(), 1);
    let body = function
        .blocks
        .iter()
        .find(|block| block.id == prepared.plan.fast_body)
        .unwrap();
    let kinds = body
        .instructions
        .iter()
        .map(|instruction| &instruction.kind)
        .collect::<Vec<_>>();
    assert!(
        kinds
            .iter()
            .any(|kind| matches!(kind, KirInstructionKind::Load { .. }))
    );
    assert!(kinds.iter().any(|kind| matches!(
        kind,
        KirInstructionKind::Binary {
            op: MirBinaryOp::Sub,
            semantics: KirArithmeticSemantics::StrictFloat,
            ..
        }
    )));
    assert!(kinds.iter().any(|kind| matches!(
        kind,
        KirInstructionKind::Binary {
            op: MirBinaryOp::Div,
            semantics: KirArithmeticSemantics::StrictFloat,
            ..
        }
    )));
    assert_eq!(
        kinds
            .iter()
            .filter(|kind| matches!(kind, KirInstructionKind::Store { .. }))
            .count(),
        1
    );
    assert!(
        !kinds
            .iter()
            .any(|kind| matches!(kind, KirInstructionKind::Compare { .. }))
    );

    let function = prepared
        .trial
        .module()
        .functions
        .iter()
        .find(|function| function.id == candidates[0].function)
        .unwrap();
    let dispatch = function
        .blocks
        .iter()
        .find(|block| block.id == prepared.plan.dispatch)
        .unwrap();
    let KirTerminator::Branch { else_edge, .. } = &dispatch.terminator else {
        panic!("range dispatcher must branch");
    };
    let vector_preheader = function
        .blocks
        .iter()
        .find(|block| block.id == else_edge.target)
        .expect("nonzero path has a dedicated preheader");
    assert_ne!(vector_preheader.id, prepared.plan.fast_header);
    assert!(!vector_preheader.params.is_empty());
    assert!(
        vector_preheader
            .params
            .iter()
            .all(|param| param.slot != "j")
    );
    assert!(vector_preheader.instructions.is_empty());
    assert!(matches!(
        &vector_preheader.terminator,
        KirTerminator::Jump { edge }
            if edge.target == prepared.plan.fast_header
                && edge.args.len() == function.blocks.iter().find(|block| block.id == prepared.plan.fast_header).unwrap().params.len()
                && edge.memory_args.len() == function.blocks.iter().find(|block| block.id == prepared.plan.fast_header).unwrap().memory_params.len()
    ));
    let fast_header = function
        .blocks
        .iter()
        .find(|block| block.id == prepared.plan.fast_header)
        .unwrap();
    let induction_slot = fast_header
        .params
        .iter()
        .position(|param| param.slot == "j")
        .unwrap();
    let KirTerminator::Jump { edge } = &vector_preheader.terminator else {
        panic!("fast preheader jump")
    };
    let initial_induction = edge.args[induction_slot];
    assert!(function.blocks.iter().flat_map(|block| &block.instructions).any(
        |instruction| instruction.results.iter().any(|result| result.value == initial_induction)
            && matches!(instruction.kind, KirInstructionKind::ConstInt { ref value } if value == "0")
    ));
    assert_eq!(else_edge.args.len(), vector_preheader.params.len());
    assert_eq!(
        else_edge.memory_args.len(),
        vector_preheader.memory_params.len()
    );

    let discovery = discover_vectorization_candidates(&prepared.trial);
    let vector_candidates = discovery
        .candidates
        .into_iter()
        .filter(|candidate| {
            candidate.function == candidates[0].function
                && candidate.header == prepared.plan.fast_header
        })
        .collect::<Vec<_>>();
    let vector_candidate = vector_candidates
        .iter()
        .find(|candidate| {
            candidate.vf == 2
                && candidate.uf == 4
                && candidate.wasm_affine.as_ref().is_some_and(|affine| {
                    affine.accesses.len() == 2 && affine.range_requirements.len() == 2
                })
        })
        .expect("dedicated preheader must expose a strict f64x2 direct-map UF4 candidate");
    let vectorized = prepare_vectorization_trial(&prepared.trial, vector_candidate)
        .expect("strict direct-map vector materialization");
    check_vectorization_trial_independently(
        &prepared.trial,
        &vectorized.trial,
        &vectorized.plan,
        &vectorized.charge,
    )
    .expect("independent strict vector check");

    let mut wrong_vector_root = vectorized.trial.clone();
    let output_parameter = wrong_vector_root.module().functions[0]
        .params
        .iter()
        .find(|param| param.name == "out")
        .unwrap()
        .value;
    let vector_load = wrong_vector_root.module_mut().functions[0]
        .blocks
        .iter_mut()
        .flat_map(|block| &mut block.instructions)
        .find(|instruction| matches!(instruction.kind, KirInstructionKind::VectorLoad { .. }))
        .unwrap();
    if let KirInstructionKind::VectorLoad { access, .. } = &mut vector_load.kind {
        access.slice = output_parameter;
    }
    assert!(
        check_vectorization_trial_independently(
            &prepared.trial,
            &wrong_vector_root,
            &vectorized.plan,
            &vectorized.charge,
        )
        .is_err(),
        "slice forwarding equivalence must not accept a different input/output root"
    );

    let mut wrong_preheader_forward = vectorized.trial.clone();
    let wrong_function = wrong_preheader_forward
        .module_mut()
        .functions
        .iter_mut()
        .find(|function| function.id == candidates[0].function)
        .unwrap();
    let wrong_input_root = wrong_function
        .params
        .iter()
        .find(|param| param.name == "out")
        .unwrap()
        .value;
    let input_slot = {
        let dispatch = wrong_function
            .blocks
            .iter()
            .find(|block| block.id == prepared.plan.dispatch)
            .unwrap();
        let KirTerminator::Branch { else_edge, .. } = &dispatch.terminator else {
            panic!("range dispatcher must branch")
        };
        wrong_function
            .blocks
            .iter()
            .find(|block| block.id == else_edge.target)
            .unwrap()
            .params
            .iter()
            .position(|param| param.slot == "input")
            .unwrap()
    };
    let dispatch = wrong_function
        .blocks
        .iter_mut()
        .find(|block| block.id == prepared.plan.dispatch)
        .unwrap();
    let KirTerminator::Branch { else_edge, .. } = &mut dispatch.terminator else {
        panic!("range dispatcher must branch")
    };
    else_edge.args[input_slot] = wrong_input_root;
    assert!(
        validate_kir_module(wrong_preheader_forward.module())
            .errors
            .is_empty()
    );
    assert!(
        check_vectorization_trial_independently(
            &prepared.trial,
            &wrong_preheader_forward,
            &vectorized.plan,
            &vectorized.charge,
        )
        .is_err(),
        "a valid but non-source preheader slice forwarding must be rejected"
    );
}

#[test]
fn normalization_unswitch_source_recognizer_should_reject_semantic_mutations() {
    for invalid in [
        NORMALIZATION.replace("maximum - minimum", "minimum - maximum"),
        NORMALIZATION.replace("range == 0.0", "range > 0.0"),
        NORMALIZATION.replace("input[j] - minimum", "input[j] - maximum"),
        NORMALIZATION.replace("j < input.len", "j < out.len"),
    ] {
        let state = wasm_state(&invalid, KirOptimizationLevel::O2);
        assert!(
            discover_normalization_unswitch_candidates(&state).is_empty(),
            "mutated source was accepted:\n{invalid}\n{}",
            print_kir_module(state.module())
        );
    }
}

#[test]
fn normalization_unswitch_recognizer_should_reject_same_region_stale_memory_forwarding() {
    let before = wasm_state(NORMALIZATION, KirOptimizationLevel::O2);
    let candidate = discover_normalization_unswitch_candidates(&before)
        .into_iter()
        .next()
        .expect("normalization candidate");
    let prepared = prepare_normalization_unswitch_trial(&before, &candidate).unwrap();
    let source_function = before
        .module()
        .functions
        .iter()
        .find(|function| function.id == candidate.function)
        .unwrap();
    let descriptor = analyze_canonical_loops(source_function)
        .loops
        .into_iter()
        .find(|loop_info| loop_info.header == candidate.header)
        .unwrap();
    let preheader_id = descriptor.preheader.unwrap();
    let header = source_function
        .blocks
        .iter()
        .find(|block| block.id == candidate.header)
        .unwrap();
    let stale = header
        .memory_params
        .iter()
        .find_map(|param| {
            source_function
                .initial_memory
                .iter()
                .find(|initial| initial.region == param.region)
                .filter(|initial| initial.version != param.version)
                .map(|initial| (param.region, initial.version))
        })
        .expect("normalization scan must advance at least one source memory region");
    let edge_sites = [
        (preheader_id, candidate.header, "entry"),
        (prepared.plan.zero_arm, prepared.plan.latch, "zero-arm join"),
        (
            prepared.plan.false_arm,
            prepared.plan.latch,
            "false-arm join",
        ),
        (prepared.plan.latch, candidate.header, "loop backedge"),
    ];
    for (source_id, target_id, label) in edge_sites {
        let mut state = before.clone();
        let function = state
            .module_mut()
            .functions
            .iter_mut()
            .find(|function| function.id == candidate.function)
            .unwrap();
        let memory_index = function
            .blocks
            .iter()
            .find(|block| block.id == target_id)
            .unwrap()
            .memory_params
            .iter()
            .position(|param| param.region == stale.0)
            .expect("target carries the stale same-region memory");
        let source = function
            .blocks
            .iter_mut()
            .find(|block| block.id == source_id)
            .unwrap();
        let edge = match &mut source.terminator {
            KirTerminator::Jump { edge } if edge.target == target_id => edge,
            _ => panic!("{label} source must jump directly to its target"),
        };
        edge.memory_args[memory_index] = stale.1;

        assert!(
            validate_kir_module(state.module()).errors.is_empty(),
            "same-region stale {label} edge remains valid KIR"
        );
        assert!(
            discover_normalization_unswitch_candidates(&state).is_empty(),
            "proposer accepted same-region stale {label} MemorySSA"
        );
        let mut stale_plan = prepared.plan.clone();
        stale_plan.pre_state_digest = state.kir_digest();
        assert!(
            check_normalization_unswitch_independently(
                &state,
                &prepared.trial,
                &stale_plan,
                &prepared.charge,
            )
            .is_err(),
            "independent checker accepted same-region stale {label} MemorySSA"
        );
    }
}

#[test]
fn normalization_unswitch_checker_should_reject_dispatch_body_and_memory_mutations() {
    let before = wasm_state(NORMALIZATION, KirOptimizationLevel::O2);
    let candidate = discover_normalization_unswitch_candidates(&before).remove(0);
    let prepared = prepare_normalization_unswitch_trial(&before, &candidate).unwrap();
    let verify = |trial: &KirVerifiedProgramState| {
        check_normalization_unswitch_independently(&before, trial, &prepared.plan, &prepared.charge)
    };

    let mut swapped_dispatch = prepared.trial.clone();
    let dispatch = swapped_dispatch.module_mut().functions[0]
        .blocks
        .iter_mut()
        .find(|block| block.id == prepared.plan.preheader)
        .unwrap();
    let KirTerminator::Branch {
        then_edge,
        else_edge,
        ..
    } = &mut dispatch.terminator
    else {
        panic!("dispatch branch")
    };
    std::mem::swap(then_edge, else_edge);
    assert!(verify(&swapped_dispatch).is_err());

    let mut changed_fast_preheader = prepared.trial.clone();
    let fast_preheader = changed_fast_preheader.module_mut().functions[0]
        .blocks
        .iter_mut()
        .find(|block| block.id == prepared.plan.fast_preheader)
        .unwrap();
    let KirTerminator::Jump { edge } = &mut fast_preheader.terminator else {
        panic!("fast preheader jump")
    };
    edge.args.swap(2, 3);
    assert!(verify(&changed_fast_preheader).is_err());

    let mut bypass_dedicated_exit = prepared.trial.clone();
    let fast_header = bypass_dedicated_exit.module_mut().functions[0]
        .blocks
        .iter_mut()
        .find(|block| block.id == prepared.plan.fast_header)
        .unwrap();
    let KirTerminator::Branch { else_edge, .. } = &mut fast_header.terminator else {
        panic!("fast loop exit")
    };
    else_edge.target = prepared.plan.exit;
    assert!(verify(&bypass_dedicated_exit).is_err());

    let mut corrupted_exit_memory = prepared.trial.clone();
    let fast_exit = corrupted_exit_memory.module_mut().functions[0]
        .blocks
        .iter_mut()
        .find(|block| block.id == prepared.plan.fast_exit)
        .unwrap();
    let KirTerminator::Jump { edge } = &mut fast_exit.terminator else {
        panic!("fast exit forwarding jump")
    };
    edge.memory_args.reverse();
    assert!(verify(&corrupted_exit_memory).is_err());

    let mut changed_threshold = prepared.trial.clone();
    let gate = changed_threshold.module_mut().functions[0]
        .blocks
        .iter_mut()
        .find(|block| block.id == prepared.plan.preheader)
        .unwrap()
        .instructions
        .iter_mut()
        .find(|instruction| instruction.id == prepared.plan.dispatch_instructions[1])
        .unwrap();
    if let KirInstructionKind::ConstInt { value } = &mut gate.kind {
        *value = prepared.plan.minimum_trip.saturating_sub(1).to_string();
    } else {
        panic!("profile threshold constant")
    }
    assert!(verify(&changed_threshold).is_err());

    let mut changed_range_test = prepared.trial.clone();
    let predicate = changed_range_test.module_mut().functions[0]
        .blocks
        .iter_mut()
        .find(|block| block.id == prepared.plan.dispatch)
        .unwrap()
        .instructions
        .iter_mut()
        .find(|instruction| instruction.id == prepared.plan.range_instructions[1])
        .unwrap();
    if let KirInstructionKind::Compare { op, .. } = &mut predicate.kind {
        *op = MirCompareOp::Ne;
    } else {
        panic!("range equality predicate")
    }
    assert!(verify(&changed_range_test).is_err());

    let mut changed_divisor = prepared.trial.clone();
    let fast_body = changed_divisor.module_mut().functions[0]
        .blocks
        .iter_mut()
        .find(|block| block.id == prepared.plan.fast_body)
        .unwrap();
    let div = fast_body
        .instructions
        .iter_mut()
        .find(|instruction| {
            matches!(
                instruction.kind,
                KirInstructionKind::Binary {
                    op: MirBinaryOp::Div,
                    ..
                }
            )
        })
        .unwrap();
    if let KirInstructionKind::Binary { op, .. } = &mut div.kind {
        *op = MirBinaryOp::Mul;
    }
    assert!(verify(&changed_divisor).is_err());

    let mut changed_backedge = prepared.trial.clone();
    let fast_body = changed_backedge.module_mut().functions[0]
        .blocks
        .iter_mut()
        .find(|block| block.id == prepared.plan.fast_body)
        .unwrap();
    let induction_slot = fast_body
        .params
        .iter()
        .position(|param| param.slot == "j")
        .unwrap();
    let unchanged_induction = fast_body.params[induction_slot].value;
    let KirTerminator::Jump { edge } = &mut fast_body.terminator else {
        panic!("fast loop backedge")
    };
    edge.args[induction_slot] = unchanged_induction;
    assert!(verify(&changed_backedge).is_err());

    let mut changed_memory = prepared.trial.clone();
    let load = changed_memory.module_mut().functions[0]
        .blocks
        .iter_mut()
        .find(|block| block.id == prepared.plan.fast_body)
        .unwrap()
        .instructions
        .iter_mut()
        .find(|instruction| matches!(instruction.kind, KirInstructionKind::Load { .. }))
        .unwrap();
    load.memory.as_mut().unwrap().input = MemoryVersionId::from_index(u32::MAX);
    assert!(verify(&changed_memory).is_err());
}

#[test]
fn normalization_unswitch_frontier_should_commit_once_with_the_original_loop_fallback() {
    let mut state = wasm_state(NORMALIZATION, KirOptimizationLevel::O2);
    let original = state.clone();
    let candidate = discover_normalization_unswitch_candidates(&state).remove(0);
    let mut audit = KirOptimizationAuditState::for_module(state.module());
    let result = run_normalization_unswitch_frontier(&mut state, &mut audit).unwrap();
    assert_eq!(result.accepted, 1);
    assert_eq!(result.rejected, 0);
    assert!(result.fallbacks.is_empty());
    let old_loop = original.module().functions[0]
        .blocks
        .iter()
        .find(|block| block.id == candidate.header)
        .unwrap();
    let retained = state.module().functions[0]
        .blocks
        .iter()
        .find(|block| block.id == candidate.header)
        .unwrap();
    assert_eq!(
        old_loop, retained,
        "the full scalar loop must remain available"
    );
    assert!(discover_normalization_unswitch_candidates(&state).is_empty());
    assert_eq!(audit.accepted(), 1);
    assert_eq!(audit.rejected(), 0);
    assert_eq!(audit.attempts().len(), 1);
    assert_eq!(
        audit.attempts()[0].key,
        CandidateKey::LoopFrontier {
            function: candidate.function,
            loop_id: candidate.loop_id,
            kind: LoopCandidateKind::NormalizationUnswitch,
            variant: LoopCandidateVariant::Scalar,
            vf: 1,
            uf: 1,
        }
    );
}

#[test]
fn normalization_unswitch_o3_pipeline_should_emit_exact_baseline_and_simd_wasm() {
    if !crate::support::command::node_available() {
        return;
    }
    let mut modules = Vec::new();
    for features in [KirWasmFeatures::Baseline, KirWasmFeatures::Simd128] {
        let profile = KirTargetProfile::webassembly_with_features(features);
        let compile = |level| {
            let checked = check(&SourceFile::new("normalization-unswitch.ck", NORMALIZATION));
            assert_eq!(checked.diagnostics, []);
            let mir = lower_to_mir(&checked.checked_program).unwrap();
            let module = build_kir_module_with_profile(
                &mir,
                KirBuildConfig {
                    consumer: KirConsumer::WebAssembly,
                    overflow_mode: KirOverflowMode::Unchecked,
                    bounds_mode: KirBoundsMode::Unchecked,
                    sanitizer_mode: KirSanitizerMode::Disabled,
                },
                profile.clone(),
            )
            .unwrap();
            let facts = import_contract_facts(&module, &checked.checked_program, 0).unwrap();
            let result = run_kir_pass_pipeline(module, level, Some(&facts));
            assert!(result.errors.is_empty(), "{:?}", result.errors);
            result.artifact.unwrap()
        };
        let before = compile(KirOptimizationLevel::O2);
        let after = compile(KirOptimizationLevel::O3);
        let printed = print_kir_module(&after);
        let has_vector_memory = after.functions.iter().any(|function| {
            function.blocks.iter().any(|block| {
                block.instructions.iter().any(|instruction| {
                    matches!(
                        instruction.kind,
                        KirInstructionKind::VectorLoad { .. }
                            | KirInstructionKind::VectorStore { .. }
                    )
                })
            })
        });
        assert_eq!(
            has_vector_memory,
            features == KirWasmFeatures::Simd128,
            "O3 normalization vectorization must match the selected WASM feature profile:\n{printed}"
        );
        assert!(
            printed.contains("normalization.nonzero.header"),
            "production O3 pipeline did not commit the scalar false-path loop:\n{printed}"
        );
        modules.push(emit_wasm_kir_module(&before, EmitWasmOptions { opt_level: 3 }).unwrap());
        modules.push(emit_wasm_kir_module(&after, EmitWasmOptions { opt_level: 3 }).unwrap());
    }
    let script = r#"
      const assert=require('node:assert/strict');
      const modules=JSON.parse(process.argv[1]);
      const cases=[
        [0n,0n,0n,0n],
        [0x8000000000000000n,0n,0x8000000000000000n,0n],
        [0x7ff8000000000123n,0n,0x3ff0000000000000n,0xbff0000000000000n]
      ];
      function run(bytes,values){
        const wasm=new WebAssembly.Instance(new WebAssembly.Module(Uint8Array.from(bytes))).exports;
        const raw=new Uint8Array(wasm.memory.buffer);raw.fill(0xa5);const view=new DataView(wasm.memory.buffer);
        values.forEach((bits,i)=>view.setBigUint64(1024+i*8,bits,true));
        wasm.normalize(1024,values.length*8,8192,values.length*8);
        return Buffer.from(raw).toString('hex');
      }
      for(let profile=0;profile<2;profile++)for(const values of cases)
        assert.equal(run(modules[profile*2+1],values),run(modules[profile*2],values));
    "#;
    let output = std::process::Command::new("node")
        .args(["-e", script, &serde_json::to_string(&modules).unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn normalization_unswitch_wasm_should_preserve_nan_signed_zero_and_trap_prefix() {
    if !crate::support::command::node_available() {
        return;
    }
    let before = wasm_state(NORMALIZATION, KirOptimizationLevel::O2);
    let candidate = discover_normalization_unswitch_candidates(&before).remove(0);
    let prepared = prepare_normalization_unswitch_trial(&before, &candidate).unwrap();
    let vector_candidate = discover_vectorization_candidates(&prepared.trial)
        .candidates
        .into_iter()
        .find(|candidate| {
            candidate.function == prepared.plan.function
                && candidate.header == prepared.plan.fast_header
                && candidate.vf == 2
                && candidate.uf == 4
        })
        .expect("normalization false arm has checked strict f64x2 UF4 candidate");
    let vectorized = prepare_vectorization_trial(&prepared.trial, &vector_candidate).unwrap();
    let modules = [
        before.module(),
        prepared.trial.module(),
        vectorized.trial.module(),
    ]
    .map(|module| emit_wasm_kir_module(module, EmitWasmOptions { opt_level: 3 }).unwrap());
    let script = r#"
      const assert=require('node:assert/strict');
      const modules=JSON.parse(process.argv[1]);
      const patterns=[
        0n, 0x8000000000000000n, 0x7ff8000000000123n,
        0x7ff0000000000000n, 0xfff0000000000000n,
        1n, 0x3ff0000000000000n, 0xc004000000000000n
      ];
      function run(bytes, values, fail) {
        const wasm=new WebAssembly.Instance(new WebAssembly.Module(Uint8Array.from(bytes))).exports;
        const raw=new Uint8Array(wasm.memory.buffer); raw.fill(0xa5);
        const view=new DataView(wasm.memory.buffer);
        const input=fail==='input'?raw.length-Math.ceil(values.length/2)*8:1024;
        const output=fail==='out'?raw.length-8:8192;
        values.slice(0,fail==='input'?Math.ceil(values.length/2):values.length)
          .forEach((bits,i)=>view.setBigUint64(input+i*8,bits,true));
        let trapped=false;
        try { wasm.normalize(input,values.length*8,output,values.length*8); }
        catch(e) { assert(e instanceof WebAssembly.RuntimeError); trapped=true; }
        return {trapped, memory:Buffer.from(raw).toString('hex')};
      }
      const cases=[
        [0n], [0n,0n,0n,0n],
        [0x8000000000000000n,0n,0x8000000000000000n],
        [0x7ff8000000000123n,0n,0x3ff0000000000000n],
        [0x3ff0000000000000n,0x7ff8000000000123n,0xbff0000000000000n],
        Array.from({length:32},(_,i)=>BigInt.asUintN(64,BigInt(i-13)<<48n)),
        Array.from({length:32},(_,i)=>i===11?0x7ff8000000000123n:BigInt.asUintN(64,BigInt(i-9)<<48n)),
        [0x7ff8000000000123n,...Array.from({length:31},(_,i)=>BigInt.asUintN(64,BigInt(i-9)<<48n))],
        ...Array.from({length:24},(_,n)=>Array.from({length:n%9+1},(_,i)=>patterns[(i*5+n)%patterns.length]))
      ];
      for(const values of cases) for(const fail of [null,'out','input'])
        for(let variant=1;variant<modules.length;variant++)
          assert.deepEqual(run(modules[variant],values,fail),run(modules[0],values,fail));
    "#;
    let output = std::process::Command::new("node")
        .args(["-e", script, &serde_json::to_string(&modules).unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    check_vectorization_trial_independently(
        &prepared.trial,
        &vectorized.trial,
        &vectorized.plan,
        &vectorized.charge,
    )
    .expect("normalization direct-map candidate passes independent checker");
}
