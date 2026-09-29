use calckernel::*;

const SOURCE: &str = r#"
export unsafe fn stencil(a:slice<f64>,out:slice<f64>,width:u32,height:u32)->void
contract { requires noalias(a,out); effects read(a),write(out); } {
 let row:u32=0;
 while row<height {
  let base:u32=row*width;let first:bool=row==0;let last:bool=row+1==height;
  let col:u32=0;
  while col<width {
   let index:u32=base+col;
   if first || col==0 || last || col+1==width {out[index]=0.0;}
   else {let top:u32=index-width;let bottom:u32=index+width;
    out[index]=(a[top-1]+2.0*a[top]+a[top+1]+2.0*a[index-1]+4.0*a[index]+2.0*a[index+1]+a[bottom-1]+2.0*a[bottom]+a[bottom+1])/16.0;}
   col=col+1;
  }
  row=row+1;
 }
}
"#;

fn state(source: &str) -> KirVerifiedProgramState {
    let checked = check(&SourceFile::new("stencil-vector.ck", source));
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
        KirTargetProfile::webassembly_with_features(KirWasmFeatures::Simd128),
    )
    .unwrap();
    let contracts = import_contract_facts(&module, &checked.checked_program, 0).unwrap();
    let result = run_kir_pass_pipeline(module, KirOptimizationLevel::O2, Some(&contracts));
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let mut pre = KirVerifiedProgramState::from_parts(
        result.artifact.unwrap(),
        result.contract_facts,
        result.proofs,
        result.eliminated_guards,
        0,
    )
    .unwrap();
    let candidate = discover_stencil_peel_candidates(&pre).remove(0);
    let peeled = prepare_stencil_peel_trial(&pre, &candidate).unwrap();
    check_stencil_peel_independently(&pre, &peeled.trial, &peeled.plan, &peeled.charge).unwrap();
    pre = KirVerifiedProgramState::from_parts(
        peeled.trial.module().clone(),
        peeled.trial.contract_facts().cloned(),
        peeled.trial.proofs().clone(),
        peeled.trial.eliminated_guards().to_vec(),
        0,
    )
    .unwrap();
    let mut audit = KirOptimizationAuditState::for_module(pre.module());
    assert_eq!(
        run_interior_normalize_frontier(&mut pre, &mut audit)
            .unwrap()
            .accepted,
        1
    );
    pre
}

#[test]
fn stencil_vector_should_reconstruct_all_nine_modular_origins_and_four_total_guards() {
    let pre = state(SOURCE);
    let candidates = discover_wasm_stencil_sources(&pre);
    assert_eq!(
        candidates.len(),
        1,
        "expected independently provable normalized stencil"
    );
    let candidate = &candidates[0];
    check_wasm_stencil_source_independently(&pre, candidate).unwrap();
    assert_eq!(candidate.loads.len(), 9);
    assert_eq!(candidate.ranges.len(), 3);
    assert_eq!(candidate.minimum_trip, 4);
    assert_eq!(
        candidate.ranges.iter().map(|r| r.count).collect::<Vec<_>>(),
        [
            StencilRangeCount::Width,
            StencilRangeCount::ThreeWidths,
            StencilRangeCount::InteriorTrip
        ]
    );
}

#[test]
fn stencil_vector_checker_should_reject_forged_envelopes_and_grid() {
    let pre = state(SOURCE);
    let source = discover_wasm_stencil_sources(&pre).remove(0);
    let reject = |candidate: &WasmStencilSource| {
        assert!(check_wasm_stencil_source_independently(&pre, candidate).is_err())
    };
    let mut candidate = source.clone();
    candidate.ranges[1].count = StencilRangeCount::InteriorTrip;
    reject(&candidate);
    let mut candidate = source.clone();
    candidate.ranges.remove(0);
    reject(&candidate);
    let mut candidate = source.clone();
    candidate.ranges[1].start = Some(candidate.store_origin);
    reject(&candidate);
    let mut candidate = source.clone();
    candidate.ranges[2].element_bytes = 4;
    reject(&candidate);
    let mut candidate = source.clone();
    candidate.loads[0].origin = candidate.loads[1].origin;
    reject(&candidate);
    let mut candidate = source.clone();
    candidate.loads[0].column = 2;
    reject(&candidate);
    let mut candidate = source.clone();
    candidate.loads.swap(0, 1);
    reject(&candidate);
    let mut candidate = source.clone();
    candidate.minimum_trip = 2;
    reject(&candidate);
    let mut candidate = source.clone();
    candidate.width = candidate.row_base;
    reject(&candidate);
    let mut candidate = source.clone();
    candidate.source_digest.clear();
    reject(&candidate);
}

fn reverify(pre: &KirVerifiedProgramState) -> KirVerifiedProgramState {
    KirVerifiedProgramState::from_parts(
        pre.module().clone(),
        pre.contract_facts().cloned(),
        pre.proofs().clone(),
        pre.eliminated_guards().to_vec(),
        0,
    )
    .unwrap()
}

#[test]
fn stencil_vector_checker_should_reject_valid_kir_changed_backedge_state() {
    let pre = state(SOURCE);
    let source = discover_wasm_stencil_sources(&pre).remove(0);
    let header = pre.module().functions[0]
        .blocks
        .iter()
        .find(|b| b.id == source.header)
        .unwrap();
    let width_slot = header
        .params
        .iter()
        .position(|p| p.slot == "width")
        .unwrap();
    let input_slot = header.params.iter().position(|p| p.slot == "a").unwrap();
    let output_slot = header.params.iter().position(|p| p.slot == "out").unwrap();
    for change_slice in [false, true] {
        let mut bad = pre.clone();
        let body = bad.module_mut().functions[0]
            .blocks
            .iter_mut()
            .find(|b| b.id == source.body)
            .unwrap();
        let KirTerminator::Jump { edge } = &mut body.terminator else {
            panic!("backedge");
        };
        if change_slice {
            edge.args[input_slot] = edge.args[output_slot];
        } else {
            edge.args[width_slot] = source.body_induction;
        }
        let bad = reverify(&bad);
        let mut candidate = source.clone();
        candidate.source_digest = bad.kir_digest();
        assert!(check_wasm_stencil_source_independently(&bad, &candidate).is_err());
        assert!(discover_wasm_stencil_sources(&bad).is_empty());
    }
}

#[test]
fn stencil_vector_checker_should_reject_valid_kir_changed_origin_and_trip() {
    let pre = state(SOURCE);
    let source = discover_wasm_stencil_sources(&pre).remove(0);
    for result in [source.loads[0].origin, source.bound] {
        let mut bad = pre.clone();
        let instruction = bad.module_mut().functions[0]
            .blocks
            .iter_mut()
            .flat_map(|b| &mut b.instructions)
            .find(|i| i.results.iter().any(|r| r.value == result))
            .unwrap();
        let KirInstructionKind::Binary { op, .. } = &mut instruction.kind else {
            panic!("arithmetic");
        };
        *op = if *op == MirBinaryOp::Add {
            MirBinaryOp::Sub
        } else {
            MirBinaryOp::Add
        };
        let bad = reverify(&bad);
        let mut candidate = source.clone();
        candidate.source_digest = bad.kir_digest();
        assert!(check_wasm_stencil_source_independently(&bad, &candidate).is_err());
        assert!(discover_wasm_stencil_sources(&bad).is_empty());
    }
}

#[test]
fn stencil_vector_should_require_available_noalias_and_complete_grid() {
    let no_alias = state(&SOURCE.replace("requires noalias(a,out);", "requires width >= 0;"));
    assert!(discover_wasm_stencil_sources(&no_alias).is_empty());
    let duplicate = state(&SOURCE.replace("a[bottom+1]", "a[bottom]"));
    assert!(discover_wasm_stencil_sources(&duplicate).is_empty());
    let pre = state(SOURCE);
    let candidate = discover_wasm_stencil_sources(&pre).remove(0);
    let without_facts = KirVerifiedProgramState::from_parts(
        pre.module().clone(),
        None,
        pre.proofs().clone(),
        pre.eliminated_guards().to_vec(),
        0,
    )
    .unwrap();
    assert!(check_wasm_stencil_source_independently(&without_facts, &candidate).is_err());
}

// This exercises the implication needed by the envelope proof, including the
// modular-underflow cases W=0/1 and origins near the wasm32 byte limit. It does
// not assume that a slice length is a product of image dimensions.
#[test]
fn stencil_vector_range_envelopes_should_imply_nonwrapping_nine_accesses() {
    let range = |data: u32, len: u32, start: u32, count: u32| {
        u64::from(start) + u64::from(count) <= u64::from(len)
            && u64::from(data) + (u64::from(start) + u64::from(count)) * 8 <= 1_u64 << 32
    };
    let mut accepted = 0;
    for width in (0_u32..40).chain([1024, (1 << 29) - 1, 1 << 29, (1 << 29) + 1, u32::MAX]) {
        for row_base in [
            0,
            1,
            width,
            width.wrapping_mul(2),
            width.wrapping_mul(9),
            u32::MAX - 1,
            u32::MAX,
        ] {
            for data in [0, 8, 0xffff_f000] {
                for len in [0, 1, 2, 3, width, width.saturating_mul(20), u32::MAX] {
                    let n = width.wrapping_sub(2);
                    let top = row_base.wrapping_sub(width);
                    let passes = n >= 4
                        && range(data, len, 0, width)
                        && range(data, len, top, width.wrapping_mul(3))
                        && range(data, len, row_base.wrapping_add(1), n);
                    if !passes {
                        continue;
                    }
                    accepted += 1;
                    assert!((6..=1 << 29).contains(&width));
                    assert_eq!(n, width - 2);
                    assert_eq!(u64::from(top) + u64::from(width), u64::from(row_base));
                    for j in [0, n - 1] {
                        for row in -1_i32..=1 {
                            for column in 0..=2 {
                                let original = row_base
                                    .wrapping_add((row as u32).wrapping_mul(width))
                                    .wrapping_add(column)
                                    .wrapping_add(j);
                                let offset = u64::from((row + 1) as u32) * u64::from(width)
                                    + u64::from(column)
                                    + u64::from(j);
                                assert!(offset < 3 * u64::from(width));
                                assert_eq!(u64::from(original), u64::from(top) + offset);
                                assert!(range(data, len, original, 1));
                            }
                        }
                    }
                }
            }
        }
    }
    assert!(accepted > 100);
}

#[test]
fn stencil_vector_should_materialize_and_independently_verify_ten_memory_groups() {
    let pre = state(SOURCE);
    let source = discover_wasm_stencil_sources(&pre).remove(0);
    let discovery = discover_vectorization_candidates(&pre);
    let candidate = discovery
        .candidates
        .iter()
        .find(|c| c.header == source.header)
        .unwrap_or_else(|| {
            panic!(
                "stencil vector candidate missing: {:?}",
                discovery.fallbacks
            )
        });
    let prepared = prepare_vectorization_trial(&pre, candidate).unwrap();
    check_vectorization_trial_independently(
        &pre,
        &prepared.trial,
        &prepared.plan,
        &prepared.charge,
    )
    .unwrap();
    assert_eq!(prepared.plan.memory_groups.len(), 10);
    assert!(prepared.plan.broadcast_groups.is_empty());
}

#[test]
fn stencil_vector_runtime_should_preserve_strict_bits_tails_and_trap_prefixes() {
    if !crate::support::command::node_available() {
        return;
    }
    for start in [0_u32, 1, 2147483648] {
        let text = SOURCE.replace("let row:u32=0;", &format!("let row:u32={start};"));
        let pre = state(&text);
        let source = discover_wasm_stencil_sources(&pre).remove(0);
        let candidate = discover_vectorization_candidates(&pre)
            .candidates
            .into_iter()
            .find(|c| c.header == source.header)
            .unwrap();
        let prepared = prepare_vectorization_trial(&pre, &candidate).unwrap();
        check_vectorization_trial_independently(
            &pre,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge,
        )
        .unwrap();
        let checked = check(&SourceFile::new("production-stencil.ck", text));
        let mir = lower_to_mir(&checked.checked_program).unwrap();
        let module =
            build_kir_module_with_profile(&mir, pre.module().config, pre.module().profile.clone())
                .unwrap();
        let contracts = import_contract_facts(&module, &checked.checked_program, 0).unwrap();
        let production = run_kir_pass_pipeline(module, KirOptimizationLevel::O3, Some(&contracts));
        assert!(production.errors.is_empty(), "{:?}", production.errors);
        let production_module = production.artifact.unwrap();
        assert_eq!(
            production_module.functions[0]
                .blocks
                .iter()
                .flat_map(|b| &b.instructions)
                .filter(|i| matches!(
                    i.kind,
                    KirInstructionKind::VectorLoad { .. } | KirInstructionKind::VectorStore { .. }
                ))
                .count(),
            10,
            "production O3 must execute the SIMD route"
        );
        let bytes = [pre.module(), prepared.trial.module(), &production_module]
            .map(|m| emit_wasm_kir_module(m, EmitWasmOptions { opt_level: 3 }).unwrap());
        let script = r#"
            const assert=require('node:assert/strict'),modules=JSON.parse(process.argv[1]),start=Number(process.argv[2]);
            const instances=modules.map(bytes=>new WebAssembly.Instance(new WebAssembly.Module(Uint8Array.from(bytes))).exports);
            let checks=0;
            function run(w,width,height,logical,failure,finite){
                const raw=new Uint8Array(w.memory.buffer);raw.fill(0xa5);const v=new DataView(w.memory.buffer);
                let input=1024,out=8192;const pattern=[0n,0x8000000000000000n,0x7ff8000000000123n,0x7ff0000000000000n,0xfff0000000000000n,1n,0x3ff0000000000000n];
                for(let i=0;i<256;i++)if(finite)v.setFloat64(input+i*8,(i%23-11)/7,true);else v.setBigUint64(input+i*8,pattern[i%pattern.length],true);
                if(failure==='left')out=raw.length-8*width;
                if(failure==='interior')out=raw.length-8*(width+3);
                if(failure==='right')out=raw.length-8*(2*width-1);
                if(failure==='read')input=raw.length-8*(3*width-1);
                let trapped=false;try{w.stencil(input,logical,out,logical,width,height)}catch(e){assert(e instanceof WebAssembly.RuntimeError);trapped=true;}
                return{trapped,memory:Buffer.from(raw).toString('hex')};
            }
            const cases=start<2?Array.from({length:20},(_,width)=>Array.from({length:7},(_,height)=>[width,height])).flat():[5,6,7,8,9,16].map(width=>[width,start+2]);
            for(const [width,height]of cases)for(const logical of[0,256])for(const finite of[false,true]){
                for(const actual of instances.slice(1)){assert.deepEqual(run(actual,width,height,logical,null,finite),run(instances[0],width,height,logical,null,finite));checks++;}
            }
            if(start===1)for(const failure of['left','interior','right','read'])for(const width of[7,8,9,16]){
                const expected=run(instances[0],width,3,256,failure,true);assert(expected.trapped,failure);
                for(const actual of instances.slice(1)){assert.deepEqual(run(actual,width,3,256,failure,true),expected);checks++;}
            }
            console.log(JSON.stringify({checks,start}));
        "#;
        let result = std::process::Command::new("node")
            .args([
                "-e",
                script,
                &serde_json::to_string(&bytes).unwrap(),
                &start.to_string(),
            ])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        eprintln!("{}", String::from_utf8_lossy(&result.stdout));
    }
}

#[test]
fn stencil_vector_emission_checker_should_reject_false_scaled_count_and_missing_guard() {
    let pre = state(SOURCE);
    let source = discover_wasm_stencil_sources(&pre).remove(0);
    let candidate = discover_vectorization_candidates(&pre)
        .candidates
        .into_iter()
        .find(|c| c.header == source.header)
        .unwrap();
    let prepared = prepare_vectorization_trial(&pre, &candidate).unwrap();
    let mut trial = prepared.trial.clone();
    let body = trial.module_mut().functions[0]
        .blocks
        .iter_mut()
        .find(|b| b.id == source.preheader)
        .unwrap();
    let scaled=body.instructions.iter_mut().find(|i|matches!(i.kind,KirInstructionKind::Binary{op:MirBinaryOp::Mul,left,..} if left==source.width)).unwrap();
    let KirInstructionKind::Binary { op, .. } = &mut scaled.kind else {
        unreachable!();
    };
    *op = MirBinaryOp::Add;
    let trial = reverify(&trial);
    assert!(
        check_vectorization_trial_independently(&pre, &trial, &prepared.plan, &prepared.charge)
            .is_err()
    );
    let mut trial = prepared.trial.clone();
    let body = trial.module_mut().functions[0]
        .blocks
        .iter_mut()
        .find(|b| b.id == source.preheader)
        .unwrap();
    let condition = body
        .instructions
        .iter_mut()
        .find(|i| matches!(i.kind, KirInstructionKind::VersionPredicate { .. }))
        .unwrap();
    let KirInstructionKind::VersionPredicate { predicate } = &mut condition.kind else {
        unreachable!();
    };
    let index=predicate.conjuncts.iter().position(|c|matches!(c,KirVersionPredicateConjunct::WasmSliceRange{count,..} if *count==source.width)).unwrap();
    predicate.conjuncts.remove(index);
    let trial = reverify(&trial);
    assert!(
        check_vectorization_trial_independently(&pre, &trial, &prepared.plan, &prepared.charge)
            .is_err()
    );
}
