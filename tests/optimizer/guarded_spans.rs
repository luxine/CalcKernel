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

fn state() -> KirVerifiedProgramState {
    let checked = check(&SourceFile::new("guarded-spans.ck", SOURCE));
    assert!(checked.diagnostics.is_empty(), "{:?}", checked.diagnostics);
    let mir = lower_to_mir(&checked.checked_program).unwrap();
    let profile = KirTargetProfile::webassembly_with_features(KirWasmFeatures::Simd128);
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
    .unwrap();
    let contracts = import_contract_facts(&module, &checked.checked_program, 0).unwrap();
    let result = run_kir_pass_pipeline(module, KirOptimizationLevel::O2, Some(&contracts));
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let mut state = KirVerifiedProgramState::from_parts(
        result.artifact.unwrap(),
        result.contract_facts,
        result.proofs,
        result.eliminated_guards,
        0,
    )
    .unwrap();
    let candidate = discover_stencil_peel_candidates(&state).remove(0);
    let peeled = prepare_stencil_peel_trial(&state, &candidate).unwrap();
    check_stencil_peel_independently(&state, &peeled.trial, &peeled.plan, &peeled.charge).unwrap();
    state = KirVerifiedProgramState::from_parts(
        peeled.trial.module().clone(),
        peeled.trial.contract_facts().cloned(),
        peeled.trial.proofs().clone(),
        peeled.trial.eliminated_guards().to_vec(),
        0,
    )
    .unwrap();
    let mut audit = KirOptimizationAuditState::for_module(state.module());
    assert_eq!(
        run_interior_normalize_frontier(&mut state, &mut audit)
            .unwrap()
            .accepted,
        1
    );
    assert!(
        validate_kir_module(state.module()).errors.is_empty(),
        "baseline positive fixture must be valid KIR: {:?}",
        validate_kir_module(state.module()).errors
    );
    state
}

fn fixture() -> (KirVerifiedProgramState, WasmStencilSource) {
    let state = state();
    let source = discover_wasm_stencil_sources(&state)
        .into_iter()
        .next()
        .expect("source addresses and envelope");
    (state, source)
}

fn requirements(source: &WasmStencilSource) -> Vec<WasmSliceRangeRequirement> {
    source
        .ranges
        .iter()
        .map(|range| WasmSliceRangeRequirement {
            slice: range.slice,
            start: range.start,
            count: match range.count {
                StencilRangeCount::Width => WasmRangeCount::Invariant(source.width),
                StencilRangeCount::ThreeWidths => WasmRangeCount::ScaledInvariant {
                    value: source.width,
                    scale: 3,
                },
                StencilRangeCount::InteriorTrip => WasmRangeCount::TripBound(source.bound),
            },
            element_bytes: range.element_bytes,
        })
        .collect()
}

fn verify(
    function: &KirFunction,
    source: &WasmStencilSource,
    requirements: &[WasmSliceRangeRequirement],
) -> Result<GuardedSpanCoverage, String> {
    verify_guarded_span_coverage(
        function,
        source.preheader,
        source.header,
        source.body,
        source.induction,
        source.body_induction,
        source.bound,
        source.minimum_trip,
        requirements,
    )
}

fn function_copy(state: &KirVerifiedProgramState, source: &WasmStencilSource) -> KirFunction {
    state
        .module()
        .functions
        .iter()
        .find(|function| function.id == source.function)
        .unwrap()
        .clone()
}

fn valid_generic_state(source: &str) -> KirVerifiedProgramState {
    let checked = check(&SourceFile::new("guarded-affine.ck", source));
    assert!(checked.diagnostics.is_empty(), "{:?}", checked.diagnostics);
    let mir = lower_to_mir(&checked.checked_program).unwrap();
    let profile = KirTargetProfile::webassembly_with_features(KirWasmFeatures::Simd128);
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
    .unwrap();
    let contracts = import_contract_facts(&module, &checked.checked_program, 0).unwrap();
    let result = run_kir_pass_pipeline(module, KirOptimizationLevel::O2, Some(&contracts));
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let state = KirVerifiedProgramState::from_parts(
        result.artifact.unwrap(),
        result.contract_facts,
        result.proofs,
        result.eliminated_guards,
        0,
    )
    .unwrap();
    assert!(
        validate_kir_module(state.module()).errors.is_empty(),
        "positive source fixture must be valid KIR: {:?}",
        validate_kir_module(state.module()).errors
    );
    state
}

fn generic_coordinates(
    function: &KirFunction,
) -> (BlockId, BlockId, BlockId, ValueId, ValueId, ValueId) {
    let descriptor = analyze_canonical_loops(function)
        .loops
        .into_iter()
        .find(|descriptor| descriptor.innermost)
        .expect("single canonical loop");
    let induction = descriptor.induction.as_ref().unwrap();
    let header = function
        .blocks
        .iter()
        .find(|block| block.id == descriptor.header)
        .unwrap();
    let KirTerminator::Branch { then_edge, .. } = &header.terminator else {
        panic!("canonical loop header must branch");
    };
    let body = function
        .blocks
        .iter()
        .find(|block| block.id == then_edge.target)
        .unwrap();
    let body_induction = then_edge
        .args
        .iter()
        .zip(&body.params)
        .find_map(|(argument, parameter)| (*argument == induction.value).then_some(parameter.value))
        .expect("body forwards canonical induction");
    (
        descriptor.preheader.unwrap(),
        descriptor.header,
        body.id,
        induction.value,
        body_induction,
        induction.bound,
    )
}

fn generic_affine_source(read_offsets: &[&str]) -> String {
    let setup = read_offsets
        .iter()
        .filter(|offset| **offset != "far" && **offset != "0")
        .copied()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .map(|offset| format!("  let offset_{offset}: u32 = {offset};"))
        .collect::<Vec<_>>()
        .join("\n");
    let reads = read_offsets
        .iter()
        .enumerate()
        .map(|(ordinal, offset)| match *offset {
            "far" if ordinal % 2 == 0 => "a[far + i]".to_string(),
            "far" => "a[i + far]".to_string(),
            "0" => "a[i]".to_string(),
            other => format!("a[i + offset_{other}]"),
        })
        .collect::<Vec<_>>()
        .join(" + ");
    format!(
        r#"
export unsafe fn affine(a: slice<f64>, out: slice<f64>, width: u32, start: u32) -> void
contract {{ requires noalias(a, out); effects read(a), write(out); }}
{{
  let trip: u32 = width - 8;
  let far: u32 = start + width + width - width + 1;
{setup}
  let i: u32 = 0;
  while i < trip {{
    out[i] = {reads};
    i = i + 1;
  }}
}}
"#
    )
}

fn generic_ranges(function: &KirFunction, range_start: ValueId) -> [WasmSliceRangeRequirement; 3] {
    let (_, _, _, _, _, bound) = generic_coordinates(function);
    let slice = function
        .params
        .iter()
        .find(|parameter| parameter.name == "a")
        .unwrap()
        .value;
    let output = function
        .params
        .iter()
        .find(|parameter| parameter.name == "out")
        .unwrap()
        .value;
    let width = function
        .params
        .iter()
        .find(|parameter| parameter.name == "width")
        .unwrap()
        .value;
    [
        WasmSliceRangeRequirement {
            slice,
            start: None,
            count: WasmRangeCount::Invariant(width),
            element_bytes: 8,
        },
        WasmSliceRangeRequirement {
            slice,
            start: Some(range_start),
            count: WasmRangeCount::ScaledInvariant {
                value: width,
                scale: 2,
            },
            element_bytes: 8,
        },
        WasmSliceRangeRequirement {
            slice: output,
            start: None,
            count: WasmRangeCount::TripBound(bound),
            element_bytes: 8,
        },
    ]
}

fn verify_generic(function: &KirFunction) -> Result<GuardedSpanCoverage, String> {
    let (preheader, header, body, induction, body_induction, bound) = generic_coordinates(function);
    let start = function
        .params
        .iter()
        .find(|parameter| parameter.name == "start")
        .unwrap()
        .value;
    let ranges = generic_ranges(function, start);
    verify_guarded_span_coverage(
        function,
        preheader,
        header,
        body,
        induction,
        body_induction,
        bound,
        4,
        &ranges,
    )
}

fn preheader_value_for_slot(function: &KirFunction, slot: &str) -> ValueId {
    let (_, header, _, _, _, _) = generic_coordinates(function);
    let header_block = function
        .blocks
        .iter()
        .find(|block| block.id == header)
        .unwrap();
    let parameter = header_block
        .params
        .iter()
        .position(|parameter| parameter.slot == slot)
        .unwrap();
    let preheader = function
        .blocks
        .iter()
        .find(|block| block.id == generic_coordinates(function).0)
        .unwrap();
    let KirTerminator::Jump { edge } = &preheader.terminator else {
        panic!("loop preheader must jump to the header");
    };
    assert_eq!(edge.target, header);
    edge.args[parameter]
}

fn append_deep_constant_chain_before(function: &mut KirFunction, target: ValueId, width: ValueId) {
    let preheader_id = generic_coordinates(function).0;
    let target_index = function
        .blocks
        .iter()
        .find(|block| block.id == preheader_id)
        .unwrap()
        .instructions
        .iter()
        .position(|instruction| {
            instruction
                .results
                .iter()
                .any(|result| result.value == target)
        })
        .expect("target value has a preheader definition");
    let next_instruction = function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .map(|instruction| instruction.id.index())
        .max()
        .unwrap()
        + 1;
    let next_value = function
        .params
        .iter()
        .map(|parameter| parameter.value.index())
        .chain(function.blocks.iter().flat_map(|block| {
            block
                .params
                .iter()
                .map(|parameter| parameter.value.index())
                .chain(
                    block
                        .instructions
                        .iter()
                        .flat_map(|instruction| &instruction.results)
                        .map(|result| result.value.index()),
                )
        }))
        .max()
        .unwrap()
        + 1;
    let scalar_u32 = KirValueType::Scalar(MirType::Primitive(MirPrimitiveTypeName::U32));
    let one = ValueId::from_index(next_value);
    let mut chain = vec![KirInstruction {
        id: InstructionId::from_index(next_instruction),
        results: vec![KirResult {
            value: one,
            type_node: scalar_u32.clone(),
        }],
        kind: KirInstructionKind::ConstInt { value: "1".into() },
        memory: None,
        effect: None,
    }];
    let mut previous = one;
    for offset in 0..600_u32 {
        let result = ValueId::from_index(next_value + offset + 1);
        chain.push(KirInstruction {
            id: InstructionId::from_index(next_instruction + offset + 1),
            results: vec![KirResult {
                value: result,
                type_node: scalar_u32.clone(),
            }],
            kind: KirInstructionKind::Binary {
                op: MirBinaryOp::Add,
                left: previous,
                right: one,
                semantics: KirArithmeticSemantics::Modular,
            },
            memory: None,
            effect: None,
        });
        previous = result;
    }
    let preheader = function
        .blocks
        .iter_mut()
        .find(|block| block.id == preheader_id)
        .unwrap();
    preheader
        .instructions
        .splice(target_index..target_index, chain);
    let target_instruction = preheader
        .instructions
        .iter_mut()
        .find(|instruction| {
            instruction
                .results
                .iter()
                .any(|result| result.value == target)
        })
        .unwrap();
    target_instruction.kind = KirInstructionKind::Binary {
        op: MirBinaryOp::Mul,
        left: previous,
        right: width,
        semantics: KirArithmeticSemantics::Modular,
    };
}

#[test]
fn generic_guarded_spans_accept_the_existing_nine_read_source() {
    let (state, source) = fixture();
    let function = function_copy(&state, &source);
    let checked = verify(&function, &source, &requirements(&source));
    assert!(checked.is_ok(), "{checked:?}");
    let checked = checked.unwrap();
    assert_eq!(checked.accesses.len(), 10);
    assert_eq!(checked.coverage.len(), checked.accesses.len());
    assert_eq!(checked.ranges.len(), 3);
}

#[test]
fn generic_guarded_spans_rebuild_origins_without_producer_metadata() {
    let (state, source) = fixture();
    let function = function_copy(&state, &source);
    let mut forged = source.clone();
    let forged_origin = forged.store_origin;
    let instruction = forged.loads[0].instruction;
    forged.loads[0].origin = forged_origin;

    let checked = verify(&function, &forged, &requirements(&forged)).unwrap();
    assert_ne!(checked.accesses[&instruction].origin, Some(forged_origin));
}

#[test]
fn generic_guarded_spans_accept_two_five_and_seven_reads_in_any_order() {
    for offsets in [
        ["far", "3"].as_slice(),
        ["3", "far", "0", "1", "2"].as_slice(),
        ["1", "far", "4", "0", "3", "2", "5"].as_slice(),
    ] {
        let state = valid_generic_state(&generic_affine_source(offsets));
        let function = &state.module().functions[0];
        assert!(validate_kir_module(state.module()).errors.is_empty());
        let checked = verify_generic(function);
        assert!(checked.is_ok(), "{} reads: {checked:?}", offsets.len());
        assert_eq!(checked.unwrap().accesses.len(), offsets.len() + 1);
    }
}

#[test]
fn generic_guarded_spans_accept_two_and_four_width_envelopes_when_they_cover() {
    let two_width = valid_generic_state(&generic_affine_source(&["far", "3"]));
    let checked = verify_generic(&two_width.module().functions[0]);
    assert!(checked.is_ok(), "two-width source envelope: {checked:?}");
    assert_eq!(checked.unwrap().accesses.len(), 3);

    let four_width = valid_generic_state(&generic_affine_source(&[
        "1", "far", "4", "0", "3", "2", "5",
    ]));
    let checked = verify_generic(&four_width.module().functions[0]);
    assert!(
        checked.is_ok(),
        "larger affine source envelope: {checked:?}"
    );
    assert_eq!(checked.unwrap().accesses.len(), 8);
}

#[test]
fn guarded_span_proof_rejects_constant_mul_after_node_budget_exhaustion() {
    let state = valid_generic_state(&generic_affine_source(&["far"]));
    let mut function = state.module().functions[0].clone();
    let far = preheader_value_for_slot(&function, "far");
    let width = function
        .params
        .iter()
        .find(|parameter| parameter.name == "width")
        .unwrap()
        .value;
    append_deep_constant_chain_before(&mut function, far, width);
    let (preheader_id, _, body_id, _induction, body_induction, _bound) =
        generic_coordinates(&function);
    let next_instruction = function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .map(|instruction| instruction.id.index())
        .max()
        .unwrap()
        + 1;
    let next_value = function
        .params
        .iter()
        .map(|parameter| parameter.value.index())
        .chain(function.blocks.iter().flat_map(|block| {
            block
                .params
                .iter()
                .map(|parameter| parameter.value.index())
                .chain(
                    block
                        .instructions
                        .iter()
                        .flat_map(|instruction| &instruction.results)
                        .map(|result| result.value.index()),
                )
        }))
        .max()
        .unwrap()
        + 1;
    let scalar_u32 = KirValueType::Scalar(MirType::Primitive(MirPrimitiveTypeName::U32));
    let origin = ValueId::from_index(next_value);
    let index = ValueId::from_index(next_value + 1);
    let preheader = function
        .blocks
        .iter_mut()
        .find(|block| block.id == preheader_id)
        .unwrap();
    let far_definition = preheader
        .instructions
        .iter()
        .position(|instruction| instruction.results.iter().any(|result| result.value == far))
        .unwrap();
    preheader.instructions.insert(
        far_definition + 1,
        KirInstruction {
            id: InstructionId::from_index(next_instruction),
            results: vec![KirResult {
                value: origin,
                type_node: scalar_u32.clone(),
            }],
            kind: KirInstructionKind::Binary {
                op: MirBinaryOp::Sub,
                left: far,
                right: far,
                semantics: KirArithmeticSemantics::Modular,
            },
            memory: None,
            effect: None,
        },
    );
    let body = function
        .blocks
        .iter_mut()
        .find(|block| block.id == body_id)
        .unwrap();
    let load_position = body
        .instructions
        .iter()
        .position(|instruction| matches!(instruction.kind, KirInstructionKind::Load { .. }))
        .unwrap();
    body.instructions.insert(
        load_position,
        KirInstruction {
            id: InstructionId::from_index(next_instruction + 1),
            results: vec![KirResult {
                value: index,
                type_node: scalar_u32,
            }],
            kind: KirInstructionKind::Binary {
                op: MirBinaryOp::Add,
                left: origin,
                right: body_induction,
                semantics: KirArithmeticSemantics::Modular,
            },
            memory: None,
            effect: None,
        },
    );
    let load = body
        .instructions
        .iter_mut()
        .find(|instruction| matches!(instruction.kind, KirInstructionKind::Load { .. }))
        .unwrap();
    let KirInstructionKind::Load { place } = &mut load.kind else {
        unreachable!();
    };
    let KirPlace::SliceIndex {
        index: load_index, ..
    } = place.as_mut()
    else {
        unreachable!();
    };
    *load_index = index;
    let mut module = state.module().clone();
    module.functions[0] = function.clone();
    assert!(
        validate_kir_module(&module).errors.is_empty(),
        "budget regression fixture must be valid KIR: {:?}",
        validate_kir_module(&module).errors
    );

    let (preheader, header, body, induction, body_induction, bound) =
        generic_coordinates(&function);
    let body_block = function
        .blocks
        .iter()
        .find(|block| block.id == body)
        .unwrap();
    let a = body_block
        .params
        .iter()
        .find(|parameter| parameter.slot == "a")
        .unwrap()
        .value;
    let out = body_block
        .params
        .iter()
        .find(|parameter| parameter.slot == "out")
        .unwrap()
        .value;
    let ranges = [
        WasmSliceRangeRequirement {
            slice: a,
            start: None,
            count: WasmRangeCount::TripBound(bound),
            element_bytes: 8,
        },
        WasmSliceRangeRequirement {
            slice: out,
            start: None,
            count: WasmRangeCount::TripBound(bound),
            element_bytes: 8,
        },
    ];
    let checked = verify_guarded_span_coverage(
        &function,
        preheader,
        header,
        body,
        induction,
        body_induction,
        bound,
        4,
        &ranges,
    );
    assert!(
        checked
            .as_ref()
            .is_err_and(|error| error.contains("budget")),
        "constant traversal budget exhaustion must fail closed, got {checked:?}"
    );
}

#[test]
fn generic_guarded_spans_reject_ranges_that_do_not_cover_the_source() {
    let (state, source) = fixture();
    let function = function_copy(&state, &source);
    let valid = requirements(&source);

    let mut outside = valid.clone();
    outside[1].start = Some(source.store_origin);
    assert!(verify(&function, &source, &outside).is_err());

    let mut one_outside = valid.clone();
    one_outside[1].start = Some(source.loads[1].origin);
    assert!(verify(&function, &source, &one_outside).is_err());

    let mut wrong_scale = valid.clone();
    wrong_scale[1].count = WasmRangeCount::ScaledInvariant {
        value: source.width,
        scale: 2,
    };
    assert!(verify(&function, &source, &wrong_scale).is_err());

    let mut wrapping_scale = valid.clone();
    wrapping_scale[1].count = WasmRangeCount::ScaledInvariant {
        value: source.width,
        scale: u32::MAX,
    };
    assert!(verify(&function, &source, &wrapping_scale).is_err());

    let mut missing_width = valid.clone();
    missing_width.remove(0);
    assert!(verify(&function, &source, &missing_width).is_err());

    let mut swapped_roots = valid.clone();
    let first_slice = swapped_roots[0].slice;
    swapped_roots[0].slice = swapped_roots[2].slice;
    swapped_roots[2].slice = first_slice;
    assert!(verify(&function, &source, &swapped_roots).is_err());

    let mut wrong_slice = valid.clone();
    wrong_slice[1].slice = source.output;
    assert!(verify(&function, &source, &wrong_slice).is_err());

    let mut wrong_iv = source.clone();
    wrong_iv.body_induction = source.width;
    assert!(verify(&function, &wrong_iv, &valid).is_err());

    let mut negative_offset = valid.clone();
    negative_offset[1].start = Some(source.store_origin);
    assert!(verify(&function, &source, &negative_offset).is_err());

    let mut over_budget = valid.clone();
    over_budget.push(valid[0]);
    assert!(verify(&function, &source, &over_budget).is_err());
}

#[test]
fn generic_guarded_spans_require_pure_closed_address_setup() {
    let (state, source) = fixture();
    let base = function_copy(&state, &source);
    let valid = verify(&base, &source, &requirements(&source)).unwrap();
    let setup_id = *valid.setup.first().expect("address setup");

    let mut effectful = base.clone();
    let setup = effectful
        .blocks
        .iter_mut()
        .flat_map(|block| &mut block.instructions)
        .find(|instruction| instruction.id == setup_id)
        .unwrap();
    setup.effect = Some(KirOrderedEffect {
        order: 0,
        kind: KirEffectKind::Call,
    });
    assert!(verify(&effectful, &source, &requirements(&source)).is_err());

    let setup_value = base
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .find(|instruction| instruction.id == setup_id)
        .and_then(|instruction| instruction.results.first())
        .unwrap();
    let next_instruction = base
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .map(|instruction| instruction.id.index())
        .max()
        .unwrap()
        + 1;
    let next_value = base
        .params
        .iter()
        .map(|parameter| parameter.value.index())
        .chain(base.blocks.iter().flat_map(|block| {
            block
                .params
                .iter()
                .map(|parameter| parameter.value.index())
                .chain(
                    block
                        .instructions
                        .iter()
                        .flat_map(|instruction| &instruction.results)
                        .map(|result| result.value.index()),
                )
        }))
        .max()
        .unwrap()
        + 1;
    let mut escaping = base.clone();
    escaping
        .blocks
        .iter_mut()
        .find(|block| block.id == source.body)
        .unwrap()
        .instructions
        .push(KirInstruction {
            id: InstructionId::from_index(next_instruction),
            results: vec![KirResult {
                value: ValueId::from_index(next_value),
                type_node: setup_value.type_node.clone(),
            }],
            kind: KirInstructionKind::Copy {
                value: setup_value.value,
            },
            memory: None,
            effect: None,
        });
    assert!(verify(&escaping, &source, &requirements(&source)).is_err());
}
