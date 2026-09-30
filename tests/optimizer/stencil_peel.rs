use calckernel::{
    KirBoundsMode, KirBuildConfig, KirConsumer, KirOptimizationLevel, KirOverflowMode,
    KirSanitizerMode, KirVerifiedProgramState, SourceFile, build_kir_module,
    build_kir_module_with_profile, check, discover_stencil_peel_candidates, import_contract_facts,
    lower_to_mir, prepare_stencil_peel_trial, run_kir_pass_pipeline,
};

// No contract grants nonaliasing or a relationship between dimensions and lengths.
const STENCIL: &str = r#"
export fn stencil(input: slice<f64>, out: slice<f64>, width: u32, height: u32) -> void {
  let row: u32 = 0;
  while row < height {
    let row_offset: u32 = row * width;
    let first: bool = row == 0;
    let last: bool = row + 1 == height;
    let col: u32 = 0;
    while col < width {
      let index: u32 = row_offset + col;
      if first || col == 0 || last || col + 1 == width {
        out[index] = 0.0;
      } else {
        out[index] = (input[index - width - 1] + input[index] * 2.0
          + input[index + width + 1]) / 4.0;
      }
      col = col + 1;
    }
    row = row + 1;
  }
}
"#;

const NINE_POINT_STENCIL: &str = r#"
export unsafe fn stencil(input: slice<f64>, out: slice<f64>, width: u32, height: u32)
-> void contract {
  requires width != 0 && height != 0;
  requires input.len == out.len;
  requires width <= input.len && height <= input.len;
  requires noalias(input, out);
  effects read(input), write(out);
} {
  let row: u32 = 0;
  while row < height {
    let col: u32 = 0;
    while col < width {
      let index: u32 = row * width + col;
      if row == 0 || col == 0 || row + 1 == height || col + 1 == width {
        out[index] = 0.0;
      } else {
        let top: u32 = index - width;
        let bottom: u32 = index + width;
        let weighted: f64 = input[top - 1] + input[top] * 2.0 + input[top + 1]
          + input[index - 1] * 2.0 + input[index] * 4.0 + input[index + 1] * 2.0
          + input[bottom - 1] + input[bottom] * 2.0 + input[bottom + 1];
        out[index] = weighted / 16.0;
      }
      col = col + 1;
    }
    row = row + 1;
  }
}
"#;

fn state(source: &str) -> KirVerifiedProgramState {
    state_with_config(
        source,
        calckernel::KirWasmFeatures::Baseline,
        KirConsumer::WebAssembly,
        KirOverflowMode::Unchecked,
        KirBoundsMode::Unchecked,
    )
}

fn state_with_config(
    source: &str,
    features: calckernel::KirWasmFeatures,
    consumer: KirConsumer,
    overflow_mode: KirOverflowMode,
    bounds_mode: KirBoundsMode,
) -> KirVerifiedProgramState {
    let checked = check(&SourceFile::new("stencil-peel.ck", source));
    assert!(checked.diagnostics.is_empty(), "{:?}", checked.diagnostics);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR");
    let config = KirBuildConfig {
        consumer,
        overflow_mode,
        bounds_mode,
        sanitizer_mode: KirSanitizerMode::Disabled,
    };
    let module = if consumer == KirConsumer::WebAssembly {
        calckernel::build_kir_module_with_profile(
            &mir,
            config,
            calckernel::KirTargetProfile::webassembly_with_features(features),
        )
    } else {
        build_kir_module(&mir, config)
    }
    .expect("KIR");
    let optimized = run_kir_pass_pipeline(module, KirOptimizationLevel::O2, None);
    assert!(optimized.errors.is_empty(), "{:?}", optimized.errors);
    KirVerifiedProgramState::from_parts(
        optimized.artifact.expect("O2"),
        optimized.contract_facts,
        optimized.proofs,
        optimized.eliminated_guards,
        0,
    )
    .expect("verified state")
}

#[test]
fn stencil_peel_should_discover_scalar_boundary_columns_without_alias_or_size_contracts() {
    let pre = state(STENCIL);
    let candidates = discover_stencil_peel_candidates(&pre);
    assert_eq!(
        candidates.len(),
        1,
        "expected one safely peelable inner loop:\n{}\n{:?}",
        calckernel::print_kir_module(pre.module()),
        calckernel::analyze_canonical_loops(&pre.module().functions[0])
    );
    let prepared = prepare_stencil_peel_trial(&pre, &candidates[0]).expect("peeling");
    assert_ne!(prepared.trial.module(), pre.module());
}

#[test]
fn wasm_o3_pipeline_commits_checked_boundary_peel_before_simd_discovery() {
    for features in [
        calckernel::KirWasmFeatures::Baseline,
        calckernel::KirWasmFeatures::Simd128,
    ] {
        let checked = check(&SourceFile::new(
            "nine-point-stencil.ck",
            NINE_POINT_STENCIL,
        ));
        assert_eq!(checked.diagnostics, []);
        let mir = lower_to_mir(&checked.checked_program).expect("MIR");
        let profile = calckernel::KirTargetProfile::webassembly_with_features(features);
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
        .expect("raw KIR");
        let contracts =
            import_contract_facts(&module, &checked.checked_program, 0).expect("source contracts");
        let optimized = run_kir_pass_pipeline(module, KirOptimizationLevel::O3, Some(&contracts));
        assert!(optimized.errors.is_empty(), "{:?}", optimized.errors);
        assert_eq!(optimized.stats.stencil_peeled_loops, 1, "{features:?}");
        assert!(optimized.records.iter().any(|record| {
            record.name == "stencil-boundary-peel" && record.changed && record.verified
        }));
        let module = optimized.artifact.expect("verified peeled O3 artifact");
        let function = module
            .functions
            .iter()
            .find(|function| function.name == "stencil")
            .expect("stencil export");
        assert!(
            function
                .blocks
                .iter()
                .any(|block| block.label.contains("peel"))
        );
    }
}

#[test]
fn stencil_peel_runtime_should_preserve_aliases_special_floats_and_trap_write_prefix() {
    if !crate::support::command::node_available() {
        return;
    }
    for (wrapped_row, features) in [
        (false, calckernel::KirWasmFeatures::Baseline),
        (true, calckernel::KirWasmFeatures::Baseline),
        (false, calckernel::KirWasmFeatures::Simd128),
        (true, calckernel::KirWasmFeatures::Simd128),
    ] {
        let source = if wrapped_row {
            STENCIL.replace("let row: u32 = 0;", "let row: u32 = 2147483648;")
        } else {
            STENCIL.to_string()
        };
        let pre = state_with_config(
            &source,
            features,
            KirConsumer::WebAssembly,
            KirOverflowMode::Unchecked,
            KirBoundsMode::Unchecked,
        );
        let candidates = discover_stencil_peel_candidates(&pre);
        assert_eq!(candidates.len(), 1, "missing boundary peel");
        let prepared = prepare_stencil_peel_trial(&pre, &candidates[0]).expect("peeling");
        let bytes = [pre.module(), prepared.trial.module()].map(|module| {
            calckernel::emit_wasm_kir_module(module, calckernel::EmitWasmOptions { opt_level: 3 })
                .expect("WASM")
        });
        let script = r#"
          const assert = require('node:assert/strict');
          const bytes = JSON.parse(process.argv[1]);
          const wrapped = process.argv[2] === 'true';
          const run = (binary, width, height, delta, failure) => {
            const w = new WebAssembly.Instance(new WebAssembly.Module(Uint8Array.from(binary))).exports;
            const raw = new Uint8Array(w.memory.buffer); raw.fill(0xA5);
            let input = 1024, out = delta === null ? 8192 : input + delta;
            const v = new DataView(w.memory.buffer);
            const patterns = [0n, 0x8000000000000000n, 0x7ff8000000000123n,
              0x7ff0000000000000n, 0xfff0000000000000n, 1n, 0x3ff0000000000000n];
            for (let i = 0; i < 256; i++) v.setBigUint64(input + i * 8, patterns[i % patterns.length], true);
            if (failure === 'store') out = raw.length - 16;
            if (failure === 'load') input = raw.length - 8;
            let trapped = false;
            try { w.stencil(input, 256, out, 256, width, height); }
            catch (error) { assert(error instanceof WebAssembly.RuntimeError); trapped = true; }
            return { trapped, memory: Buffer.from(raw).toString('hex') };
          };
          const cases = wrapped ? [[4, 2147483650], [5, 2147483650]]
            : Array.from({length: 10}, (_,w) => Array.from({length: 6}, (_,h) => [w,h])).flat();
          for (const [width,height] of cases) for (const delta of [null,0,8,-8])
            assert.deepEqual(run(bytes[1],width,height,delta), run(bytes[0],width,height,delta));
          if (!wrapped) for (const failure of ['store','load']) {
            const expected = run(bytes[0],5,5,null,failure);
            assert.equal(expected.trapped,true);
            assert.deepEqual(run(bytes[1],5,5,null,failure),expected);
          }
        "#;
        let output = std::process::Command::new("node")
            .args([
                "-e",
                script,
                &serde_json::to_string(&bytes).unwrap(),
                if wrapped_row { "true" } else { "false" },
            ])
            .output()
            .expect("node");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn stencil_peel_checker_should_reject_forged_paths_bounds_memory_order_and_fallback() {
    let pre = state(STENCIL);
    let candidate = discover_stencil_peel_candidates(&pre).remove(0);
    let prepared = prepare_stencil_peel_trial(&pre, &candidate).unwrap();
    let check_trial = |trial: &KirVerifiedProgramState,
                       plan: &calckernel::StencilPeelPlan,
                       charge: &calckernel::CandidateBudgetCharge| {
        calckernel::check_stencil_peel_independently(&pre, trial, plan, charge)
    };
    assert!(check_trial(&prepared.trial, &prepared.plan, &prepared.charge).is_ok());
    let mut stale = prepared.plan.clone();
    stale.pre_state_digest.push('x');
    assert!(
        check_trial(&prepared.trial, &stale, &prepared.charge).is_err(),
        "stale source accepted"
    );
    let mut path = prepared.plan.clone();
    path.phases[1].source_blocks.reverse();
    assert!(
        check_trial(&prepared.trial, &path, &prepared.charge).is_err(),
        "forged path accepted"
    );
    let mut charge = prepared.charge.clone();
    charge.checker_units += 1;
    assert!(
        check_trial(&prepared.trial, &prepared.plan, &charge).is_err(),
        "false charge accepted"
    );
    let mut bound = prepared.trial.clone();
    let header = bound.module_mut().functions[0]
        .blocks
        .iter_mut()
        .find(|b| b.id == prepared.plan.interior_header)
        .unwrap();
    let calckernel::KirInstructionKind::Compare { op, .. } = &mut header.instructions[0].kind
    else {
        panic!("compare")
    };
    *op = calckernel::MirCompareOp::Le;
    assert!(
        check_trial(&bound, &prepared.plan, &prepared.charge).is_err(),
        "overlapping right column accepted"
    );
    let mut order = prepared.trial.clone();
    let body = order.module_mut().functions[0]
        .blocks
        .iter_mut()
        .find(|b| b.id == prepared.plan.phases[1].block)
        .unwrap();
    let loads = body
        .instructions
        .iter()
        .enumerate()
        .filter_map(|(index, i)| {
            matches!(i.kind, calckernel::KirInstructionKind::Load { .. }).then_some(index)
        })
        .collect::<Vec<_>>();
    body.instructions.swap(loads[0], loads[1]);
    assert!(
        check_trial(&order, &prepared.plan, &prepared.charge).is_err(),
        "reordered loads accepted"
    );
    let mut memory = prepared.trial.clone();
    let body = memory.module_mut().functions[0]
        .blocks
        .iter_mut()
        .find(|b| b.id == prepared.plan.phases[1].block)
        .unwrap();
    let mem = body
        .instructions
        .iter_mut()
        .find_map(|i| i.memory.as_mut())
        .unwrap();
    mem.input = calckernel::MemoryVersionId::from_index(0);
    assert!(
        check_trial(&memory, &prepared.plan, &prepared.charge).is_err(),
        "forged memory accepted"
    );
    let mut fallback = prepared.trial.clone();
    let block = fallback.module_mut().functions[0]
        .blocks
        .iter_mut()
        .find(|b| b.id == prepared.plan.header)
        .unwrap();
    block.instructions.clear();
    assert!(
        check_trial(&fallback, &prepared.plan, &prepared.charge).is_err(),
        "changed fallback accepted"
    );
}

#[test]
fn stencil_peel_should_reject_neighboring_induction_and_data_dependent_shapes() {
    for source in [
        STENCIL.replace("col = col + 1;", "col = col + 2;"),
        STENCIL.replace("col < width", "col <= width"),
        STENCIL.replace("col + 1 == width", "col + 2 == width"),
        STENCIL.replace("if first ||", "if input[col] > 0.0 ||"),
        STENCIL.replace("out[index] = 0.0;", "out[index] = input[index];"),
    ] {
        let pre = state(&source);
        assert!(
            discover_stencil_peel_candidates(&pre).is_empty(),
            "unexpected peel for {source}"
        );
    }
}

#[test]
fn stencil_peel_verified_transaction_should_commit_and_rollback_all_state() {
    let mut pre = state(STENCIL);
    let candidate = discover_stencil_peel_candidates(&pre).remove(0);
    let prepared = prepare_stencil_peel_trial(&pre, &candidate).unwrap();
    let mut audit = calckernel::KirOptimizationAuditState::for_module(pre.module());
    let key = calckernel::CandidateKey::LoopFrontier {
        function: candidate.function,
        loop_id: candidate.loop_id,
        kind: calckernel::LoopCandidateKind::BoundaryPeel,
        variant: calckernel::LoopCandidateVariant::Scalar,
        vf: 1,
        uf: 1,
    };
    let before = pre.clone();
    let mut bad = prepared.trial.clone();
    bad.module_mut().functions[0]
        .blocks
        .iter_mut()
        .find(|b| b.id == prepared.plan.phases[1].block)
        .unwrap()
        .instructions
        .reverse();
    let outcome = calckernel::execute_verified_transaction(
        &mut pre,
        &mut audit,
        key.clone(),
        prepared.charge.clone(),
        move |trial| {
            *trial = bad;
            Ok(())
        },
        |before, after| {
            calckernel::check_stencil_peel_independently(
                before,
                after,
                &prepared.plan,
                &prepared.charge,
            )
        },
    );
    assert!(matches!(
        outcome,
        calckernel::TransactionOutcome::CompilerError(_)
    ));
    assert_eq!(
        pre, before,
        "rejected transaction must restore complete state"
    );
    let proposed = prepared.trial;
    // A separate audit gives this independent valid attempt its own candidate key.
    let mut audit = calckernel::KirOptimizationAuditState::for_module(pre.module());
    let outcome = calckernel::execute_verified_transaction(
        &mut pre,
        &mut audit,
        key,
        prepared.charge.clone(),
        move |trial| {
            *trial = proposed;
            Ok(())
        },
        |before, after| {
            calckernel::check_stencil_peel_independently(
                before,
                after,
                &prepared.plan,
                &prepared.charge,
            )
        },
    );
    assert_eq!(outcome, calckernel::TransactionOutcome::Committed);
    assert_ne!(pre.module(), before.module());
}

#[test]
fn stencil_peel_checker_should_reconstruct_source_bounds_and_backedge_invariants_independently() {
    let pre = state(STENCIL);
    let candidate = discover_stencil_peel_candidates(&pre).remove(0);
    let prepared = prepare_stencil_peel_trial(&pre, &candidate).unwrap();
    let source_header = pre.module().functions[0]
        .blocks
        .iter()
        .find(|b| b.id == candidate.header)
        .unwrap();
    let calckernel::KirInstructionKind::Compare {
        left: column,
        right: width,
        ..
    } = source_header.instructions[0].kind
    else {
        panic!("header")
    };
    let width_index = source_header
        .params
        .iter()
        .position(|p| p.value == width)
        .unwrap();
    let col_index = source_header
        .params
        .iter()
        .position(|p| p.value == column)
        .unwrap();
    let latch = calckernel::analyze_canonical_loops(&pre.module().functions[0])
        .loops
        .into_iter()
        .find(|l| l.header == candidate.header)
        .unwrap()
        .latch
        .unwrap();
    let mut changed_pre = pre.clone();
    let mut changed_trial = prepared.trial.clone();
    for state in [&mut changed_pre, &mut changed_trial] {
        let block = state.module_mut().functions[0]
            .blocks
            .iter_mut()
            .find(|b| b.id == latch)
            .unwrap();
        let calckernel::KirTerminator::Jump { edge } = &mut block.terminator else {
            panic!("latch")
        };
        edge.args[width_index] = edge.args[col_index];
    }
    assert!(
        calckernel::validate_kir_module(changed_pre.module())
            .errors
            .is_empty(),
        "mutation must remain well typed"
    );
    let mut plan = prepared.plan.clone();
    plan.pre_state_digest = changed_pre.kir_digest();
    let error = calckernel::check_stencil_peel_independently(
        &changed_pre,
        &changed_trial,
        &plan,
        &prepared.charge,
    )
    .unwrap_err();
    assert!(
        format!("{error:?}").contains("source induction or invariant state changes"),
        "{error:?}"
    );

    let mut changed_pre = pre.clone();
    let mut changed_trial = prepared.trial.clone();
    let boundary_compare = prepared.plan.phases[0]
        .instruction_mapping
        .iter()
        .filter_map(|(source, _)| {
            pre.module().functions[0]
                .blocks
                .iter()
                .flat_map(|b| &b.instructions)
                .find(|i| i.id == *source)
        })
        .find(|i| {
            matches!(
                i.kind,
                calckernel::KirInstructionKind::Compare {
                    op: calckernel::MirCompareOp::Eq,
                    ..
                }
            )
        })
        .unwrap()
        .id;
    for state in [&mut changed_pre, &mut changed_trial] {
        let instruction = state.module_mut().functions[0]
            .blocks
            .iter_mut()
            .flat_map(|b| &mut b.instructions)
            .find(|i| i.id == boundary_compare)
            .unwrap();
        let calckernel::KirInstructionKind::Compare { op, .. } = &mut instruction.kind else {
            panic!("boundary")
        };
        *op = calckernel::MirCompareOp::Ne;
    }
    let mut plan = prepared.plan.clone();
    plan.pre_state_digest = changed_pre.kir_digest();
    let error = calckernel::check_stencil_peel_independently(
        &changed_pre,
        &changed_trial,
        &plan,
        &prepared.charge,
    )
    .unwrap_err();
    assert!(
        format!("{error:?}").contains("checker cannot prove a branch constant"),
        "{error:?}"
    );
}

#[test]
fn stencil_peel_should_preserve_unmeasured_consumers_and_checked_build_rejection() {
    for consumer in [
        KirConsumer::C,
        KirConsumer::NativeLibrary,
        KirConsumer::NativeExecutable,
    ] {
        let source = if consumer == KirConsumer::NativeExecutable {
            format!("{STENCIL}\nfn main() -> void {{}}")
        } else {
            STENCIL.to_string()
        };
        let pre = state_with_config(
            &source,
            calckernel::KirWasmFeatures::Baseline,
            consumer,
            KirOverflowMode::Unchecked,
            KirBoundsMode::Unchecked,
        );
        assert!(
            discover_stencil_peel_candidates(&pre).is_empty(),
            "{consumer:?}"
        );
    }
    let checked = check(&SourceFile::new("checked-stencil.ck", STENCIL));
    let mir = lower_to_mir(&checked.checked_program).unwrap();
    let error = build_kir_module(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::WebAssembly,
            overflow_mode: KirOverflowMode::Checked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
    )
    .unwrap_err();
    assert!(
        error.message.contains("does not support checked overflow"),
        "{error:?}"
    );
}
