use calckernel::*;

const SOURCE: &str = r#"
export fn stencil(a: slice<f64>, out: slice<f64>, width: u32, height: u32) -> void {
  let row: u32 = 0;
  while row < height {
    let origin: u32 = row * width;
    let first: bool = row == 0;
    let last: bool = row + 1 == height;
    let col: u32 = 0;
    while col < width {
      let index: u32 = origin + col;
      if first || col == 0 || last || col + 1 == width { out[index] = 0.0; }
      else { out[index] = (a[index - width - 1] + 2.0 * a[index] + a[index + width + 1]) / 4.0; }
      col = col + 1;
    }
    row = row + 1;
  }
}
"#;

fn peeled_state(source: &str, features: KirWasmFeatures) -> KirVerifiedProgramState {
    let checked = check(&SourceFile::new("interior-normalize.ck", source));
    assert!(checked.diagnostics.is_empty(), "{:?}", checked.diagnostics);
    let mir = lower_to_mir(&checked.checked_program).unwrap();
    let module = build_kir_module_with_profile(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::WebAssembly,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
        KirTargetProfile::webassembly_with_features(features),
    )
    .unwrap();
    let result = run_kir_pass_pipeline(module, KirOptimizationLevel::O2, None);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let pre = KirVerifiedProgramState::from_parts(
        result.artifact.unwrap(),
        result.contract_facts,
        result.proofs,
        result.eliminated_guards,
        0,
    )
    .unwrap();
    let candidate = discover_stencil_peel_candidates(&pre).remove(0);
    let prepared = prepare_stencil_peel_trial(&pre, &candidate).unwrap();
    check_stencil_peel_independently(&pre, &prepared.trial, &prepared.plan, &prepared.charge)
        .unwrap();
    KirVerifiedProgramState::from_parts(
        prepared.trial.module().clone(),
        prepared.trial.contract_facts().cloned(),
        prepared.trial.proofs().clone(),
        prepared.trial.eliminated_guards().to_vec(),
        0,
    )
    .unwrap()
}

#[test]
fn interior_normalize_should_find_and_rebase_the_peeled_scalar_loop() {
    let pre = peeled_state(SOURCE, KirWasmFeatures::Baseline);
    let candidates = discover_interior_normalize_candidates(&pre);
    assert_eq!(
        candidates.len(),
        1,
        "expected the nonzero-origin peeled interior: {:?}\n{}",
        analyze_canonical_loops(&pre.module().functions[0]),
        print_kir_module(pre.module())
    );
    let prepared = prepare_interior_normalize_trial(&pre, &candidates[0]).unwrap();
    assert_ne!(pre.module(), prepared.trial.module());
}

#[test]
fn interior_normalize_runtime_should_preserve_values_aliases_wraps_and_trap_prefixes() {
    if !crate::support::command::node_available() {
        return;
    }
    for features in [KirWasmFeatures::Baseline, KirWasmFeatures::Simd128] {
        for wrapped in [false, true] {
            let source = if wrapped {
                SOURCE.replace("let row: u32 = 0;", "let row: u32 = 2147483648;")
            } else {
                SOURCE.into()
            };
            let pre = peeled_state(&source, features);
            let candidates = discover_interior_normalize_candidates(&pre);
            assert_eq!(candidates.len(), 1, "missing zero-origin normalization");
            let prepared = prepare_interior_normalize_trial(&pre, &candidates[0]).unwrap();
            let modules = [pre.module(), prepared.trial.module()]
                .map(|m| emit_wasm_kir_module(m, EmitWasmOptions { opt_level: 3 }).unwrap());
            let script = r#"
              const assert=require('node:assert/strict');const modules=JSON.parse(process.argv[1]);
              const wrapped=process.argv[2]==='true';
              function run(bytes,width,height,delta,failure){
                const w=new WebAssembly.Instance(new WebAssembly.Module(Uint8Array.from(bytes))).exports;
                const raw=new Uint8Array(w.memory.buffer);raw.fill(0xa5);const v=new DataView(w.memory.buffer);
                let input=1024,out=delta===null?8192:1024+delta;
                const patterns=[0n,0x8000000000000000n,0x7ff8000000000123n,0x7ff0000000000000n,0xfff0000000000000n,1n,0x3ff0000000000000n];
                for(let i=0;i<256;i++)v.setBigUint64(input+i*8,patterns[i%patterns.length],true);
                if(failure==='left')out=raw.length-8;if(failure==='interior')input=raw.length-8;
                let trapped=false;try{w.stencil(input,256,out,256,width,height)}catch(e){assert(e instanceof WebAssembly.RuntimeError);trapped=true;}
                return {trapped,memory:Buffer.from(raw).toString('hex')};
              }
              const cases=wrapped?[[4,2147483650],[5,2147483650]]:Array.from({length:12},(_,w)=>Array.from({length:6},(_,h)=>[w,h])).flat();
              for(const [w,h]of cases)for(const delta of[null,0,8,-8])assert.deepEqual(run(modules[1],w,h,delta),run(modules[0],w,h,delta));
              if(!wrapped)for(const failure of['left','interior']){let expected=run(modules[0],5,5,null,failure);assert(expected.trapped);assert.deepEqual(run(modules[1],5,5,null,failure),expected);}
            "#;
            let output = std::process::Command::new("node")
                .args([
                    "-e",
                    script,
                    &serde_json::to_string(&modules).unwrap(),
                    if wrapped { "true" } else { "false" },
                ])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}

#[test]
fn interior_normalize_checker_should_reject_stale_bounds_origins_order_and_exit_mutations() {
    let pre = peeled_state(SOURCE, KirWasmFeatures::Simd128);
    let candidate = discover_interior_normalize_candidates(&pre).remove(0);
    let prepared = prepare_interior_normalize_trial(&pre, &candidate).unwrap();
    let verify = |trial: &KirVerifiedProgramState, plan: &InteriorNormalizePlan| {
        check_interior_normalize_independently(&pre, trial, plan, &prepared.charge)
    };
    assert!(verify(&prepared.trial, &prepared.plan).is_ok());
    let mut plan = prepared.plan.clone();
    plan.pre_state_digest.push('x');
    assert!(
        verify(&prepared.trial, &plan).is_err(),
        "stale source accepted"
    );
    for (target, result) in [
        (prepared.plan.blocks[1], prepared.plan.trip_count),
        (prepared.plan.blocks[1], prepared.plan.addresses[0].origin),
        (prepared.plan.blocks[4], prepared.plan.exit_column),
    ] {
        let mut trial = prepared.trial.clone();
        let i = trial.module_mut().functions[0]
            .blocks
            .iter_mut()
            .find(|b| b.id == target)
            .unwrap()
            .instructions
            .iter_mut()
            .find(|i| i.results.iter().any(|r| r.value == result))
            .unwrap();
        let KirInstructionKind::Binary { op, .. } = &mut i.kind else {
            panic!("arithmetic")
        };
        *op = if *op == MirBinaryOp::Add {
            MirBinaryOp::Sub
        } else {
            MirBinaryOp::Add
        };
        assert!(
            verify(&trial, &prepared.plan).is_err(),
            "forged bound/origin/exit accepted"
        );
    }
    let mut trial = prepared.trial.clone();
    let body = trial.module_mut().functions[0]
        .blocks
        .iter_mut()
        .find(|b| b.id == prepared.plan.blocks[3])
        .unwrap();
    let loads = body
        .instructions
        .iter()
        .enumerate()
        .filter_map(|(n, i)| matches!(i.kind, KirInstructionKind::Load { .. }).then_some(n))
        .collect::<Vec<_>>();
    body.instructions.swap(loads[0], loads[1]);
    assert!(
        verify(&trial, &prepared.plan).is_err(),
        "changed memory/trap order accepted"
    );
    let mut trial = prepared.trial.clone();
    trial.module_mut().functions[0]
        .blocks
        .iter_mut()
        .find(|b| b.id == prepared.plan.preheader)
        .unwrap()
        .instructions
        .clear();
    assert!(
        verify(&trial, &prepared.plan).is_err(),
        "removed left store accepted"
    );
}

#[test]
fn interior_normalize_checker_should_reject_reused_index_for_another_source_origin() {
    let pre = peeled_state(SOURCE, KirWasmFeatures::Simd128);
    let candidate = discover_interior_normalize_candidates(&pre).remove(0);
    let prepared = prepare_interior_normalize_trial(&pre, &candidate).unwrap();
    check_interior_normalize_independently(&pre, &prepared.trial, &prepared.plan, &prepared.charge)
        .unwrap();

    let first = &prepared.plan.addresses[0];
    assert!(
        prepared.plan.addresses.iter().any(|address| {
            prepared
                .plan
                .addresses
                .iter()
                .filter(|other| other.index == address.index)
                .count()
                > 1
        }),
        "the genuine trial must exercise shared load/store index reuse"
    );
    // Select a later distinct address that has one user, so deleting its
    // builder leaves a well-formed program after redirecting that load.
    let position = prepared
        .plan
        .addresses
        .iter()
        .position(|address| {
            address.index != first.index
                && address.origin != first.origin
                && prepared
                    .plan
                    .addresses
                    .iter()
                    .filter(|other| other.index == address.index)
                    .count()
                    == 1
        })
        .unwrap();
    let later = &prepared.plan.addresses[position];
    let emitted_load = prepared
        .plan
        .instruction_mapping
        .iter()
        .find(|(source, _)| *source == later.source)
        .unwrap()
        .1;
    let mut trial = prepared.trial.clone();
    let body = trial.module_mut().functions[0]
        .blocks
        .iter_mut()
        .find(|block| block.id == prepared.plan.blocks[3])
        .unwrap();
    body.instructions
        .retain(|instruction| !instruction.results.iter().any(|r| r.value == later.index));
    let load = body
        .instructions
        .iter_mut()
        .find(|instruction| instruction.id == emitted_load)
        .unwrap();
    let KirInstructionKind::Load { place } = &mut load.kind else {
        panic!("expected a later load");
    };
    let KirPlace::SliceIndex { index, .. } = place.as_mut() else {
        panic!("slice index");
    };
    *index = first.index;

    let mut plan = prepared.plan.clone();
    plan.addresses[position].index = first.index;
    plan.after_units = kir_function_units(&trial.module().functions[0]);
    let charge = CandidateBudgetCharge::single(
        plan.function,
        plan.after_units
            .saturating_sub(plan.before_units)
            .saturating_add(24),
        plan.before_units
            .saturating_add(plan.after_units)
            .saturating_add(48),
    );
    assert!(
        validate_kir_module(trial.module()).errors.is_empty(),
        "mutant must be valid KIR"
    );
    let error = check_interior_normalize_independently(&pre, &trial, &plan, &charge).unwrap_err();
    assert!(
        format!("{error:?}").contains("reused an index for a different source index or origin"),
        "{error:?}"
    );
}

#[test]
fn interior_normalize_source_checker_should_reject_same_type_backedge_bound_changes() {
    let pre = peeled_state(SOURCE, KirWasmFeatures::Baseline);
    let candidate = discover_interior_normalize_candidates(&pre).remove(0);
    let prepared = prepare_interior_normalize_trial(&pre, &candidate).unwrap();
    let header = pre.module().functions[0]
        .blocks
        .iter()
        .find(|b| b.id == candidate.header)
        .unwrap();
    let KirInstructionKind::Compare { left: column, .. } = header.instructions[0].kind else {
        panic!("range")
    };
    let column_slot = header
        .params
        .iter()
        .position(|p| p.value == column)
        .unwrap();
    let width_slot = header
        .params
        .iter()
        .position(|p| p.slot == "width")
        .unwrap();
    let mut before = pre.clone();
    let mut after = prepared.trial.clone();
    for state in [&mut before, &mut after] {
        let b = state.module_mut().functions[0]
            .blocks
            .iter_mut()
            .find(|b| b.id == prepared.plan.source_body)
            .unwrap();
        let KirTerminator::Jump { edge } = &mut b.terminator else {
            panic!("backedge")
        };
        edge.args[width_slot] = edge.args[column_slot];
    }
    assert!(
        validate_kir_module(before.module()).errors.is_empty(),
        "source mutation must stay well typed"
    );
    let mut plan = prepared.plan.clone();
    plan.pre_state_digest = before.kir_digest();
    let error = check_interior_normalize_independently(&before, &after, &plan, &prepared.charge)
        .unwrap_err();
    assert!(
        format!("{error:?}").contains("bound or non-column state changes"),
        "{error:?}"
    );
}

#[test]
fn interior_normalize_frontier_should_commit_once_and_keep_original_scalar_fallback() {
    let mut state = peeled_state(SOURCE, KirWasmFeatures::Simd128);
    let original = state.clone();
    let candidate = discover_interior_normalize_candidates(&state).remove(0);
    let mut audit = KirOptimizationAuditState::for_module(state.module());
    let result = run_interior_normalize_frontier(&mut state, &mut audit).unwrap();
    assert_eq!(result.accepted, 1);
    assert_eq!(result.rejected, 0);
    assert!(result.fallbacks.is_empty());
    let before = original.module().functions[0]
        .blocks
        .iter()
        .find(|b| b.id == candidate.header)
        .unwrap();
    let after = state.module().functions[0]
        .blocks
        .iter()
        .find(|b| b.id == candidate.header)
        .unwrap();
    assert_eq!(before, after, "legacy interior must be retained");
    assert!(
        discover_interior_normalize_candidates(&state).is_empty(),
        "normalization must not recursively version its fallback"
    );
    assert!(
        analyze_canonical_loops(&state.module().functions[0])
            .loops
            .iter()
            .any(|d| d
                .induction
                .as_ref()
                .is_some_and(|i| i.start.to_string() == "0")
                && d.blocks.len() == 2),
        "zero-based interior must be visible to loop analysis"
    );
}

#[test]
fn interior_normalize_should_preserve_observed_address_intermediates() {
    let source = SOURCE.replace(") / 4.0;", ") / 4.0 + u32_to_f64(index);");
    let pre = peeled_state(&source, KirWasmFeatures::Baseline);
    let candidate = discover_interior_normalize_candidates(&pre).remove(0);
    let prepared = prepare_interior_normalize_trial(&pre, &candidate).unwrap();
    check_interior_normalize_independently(&pre, &prepared.trial, &prepared.plan, &prepared.charge)
        .unwrap();
    let body = pre.module().functions[0]
        .blocks
        .iter()
        .find(|b| b.id == prepared.plan.source_body)
        .unwrap();
    let cast_input = body
        .instructions
        .iter()
        .find_map(|i| match i.kind {
            KirInstructionKind::Cast { value, .. } => Some(value),
            _ => None,
        })
        .unwrap();
    let observed = body
        .instructions
        .iter()
        .find(|i| i.results.iter().any(|r| r.value == cast_input))
        .unwrap()
        .id;
    assert!(!prepared.plan.removed_instructions.contains(&observed));
    let mut forged = prepared.trial.clone();
    let mut plan = prepared.plan.clone();
    let mapping = plan
        .instruction_mapping
        .iter()
        .find(|(source, _)| *source == observed)
        .copied()
        .unwrap();
    plan.instruction_mapping.retain(|p| p != &mapping);
    plan.removed_instructions.push(observed);
    forged.module_mut().functions[0]
        .blocks
        .iter_mut()
        .find(|b| b.id == plan.blocks[3])
        .unwrap()
        .instructions
        .retain(|i| i.id != mapping.1);
    let error =
        check_interior_normalize_independently(&pre, &forged, &plan, &prepared.charge).unwrap_err();
    assert!(
        format!("{error:?}").contains("observed intermediate"),
        "{error:?}"
    );
    if !crate::support::command::node_available() {
        return;
    }
    let bytes = [pre.module(), prepared.trial.module()]
        .map(|m| emit_wasm_kir_module(m, EmitWasmOptions { opt_level: 3 }).unwrap());
    let script = r#"
      const assert=require('node:assert/strict');const modules=JSON.parse(process.argv[1]);
      function run(bytes,alias){const w=new WebAssembly.Instance(new WebAssembly.Module(Uint8Array.from(bytes))).exports;
        const a=new Float64Array(w.memory.buffer,1024,64);for(let i=0;i<a.length;i++)a[i]=i/7;
        const out=alias?1032:4096;w.stencil(1024,64,out,64,7,5);return Buffer.from(w.memory.buffer).toString('hex');}
      for(const alias of[false,true])assert.equal(run(modules[0],alias),run(modules[1],alias));
    "#;
    let output = std::process::Command::new("node")
        .args(["-e", script, &serde_json::to_string(&bytes).unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn interior_normalize_should_not_treat_a_copy_of_an_address_or_header_counter_as_invariant() {
    let mut pre = peeled_state(SOURCE, KirWasmFeatures::Baseline);
    let candidate = discover_interior_normalize_candidates(&pre).remove(0);
    let function = &pre.module().functions[0];
    let header = function
        .blocks
        .iter()
        .find(|b| b.id == candidate.header)
        .unwrap();
    let KirInstructionKind::Compare {
        left: header_column,
        ..
    } = header.instructions[0].kind
    else {
        panic!("header")
    };
    let KirTerminator::Branch { then_edge, .. } = &header.terminator else {
        panic!("body")
    };
    let body_id = then_edge.target;
    let ids = pre.ids();
    let copy_value = ValueId::from_index(ids.next_value);
    let index_value = ValueId::from_index(ids.next_value + 1);
    let body = pre.module_mut().functions[0]
        .blocks
        .iter_mut()
        .find(|b| b.id == body_id)
        .unwrap();
    let position = body
        .instructions
        .iter()
        .position(|i| matches!(i.kind, KirInstructionKind::Load { .. }))
        .unwrap();
    let KirInstructionKind::Load { place } = &mut body.instructions[position].kind else {
        panic!("load")
    };
    let KirPlace::SliceIndex { index, .. } = place.as_mut() else {
        panic!("index")
    };
    let old_index = *index;
    *index = index_value;
    let instruction = |id, value, kind| KirInstruction {
        id: InstructionId::from_index(id),
        results: vec![KirResult {
            value,
            type_node: MirType::Primitive(MirPrimitiveTypeName::U32).into(),
        }],
        kind,
        memory: None,
        effect: None,
    };
    body.instructions.splice(
        position..position,
        [
            instruction(
                ids.next_instruction,
                copy_value,
                KirInstructionKind::Copy { value: old_index },
            ),
            instruction(
                ids.next_instruction + 1,
                index_value,
                KirInstructionKind::Binary {
                    op: MirBinaryOp::Add,
                    left: copy_value,
                    right: header_column,
                    semantics: KirArithmeticSemantics::Modular,
                },
            ),
        ],
    );
    assert!(
        validate_kir_module(pre.module()).errors.is_empty(),
        "mutant must remain valid KIR"
    );
    assert!(
        discover_interior_normalize_candidates(&pre).is_empty(),
        "two varying column terms are not a unit-stride index"
    );
}
