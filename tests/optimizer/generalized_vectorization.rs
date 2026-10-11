use calckernel::{
    InstructionId, KirAlignmentClass, KirBoundsMode, KirBuildConfig, KirConsumer, KirLaneType,
    KirLegalCost, KirNativeCpuPolicy, KirOptimizationLevel, KirOverflowMode, KirProfileOperation,
    KirSanitizerMode, KirTargetProfile, KirTargetProfileBuilder, KirVerifiedProgramState,
    KirWasmFeatures, SourceFile, VectorEpilogue, VectorPredicate, build_kir_module_with_profile,
    check_vectorization_trial_independently, discover_vectorization_candidates,
    import_contract_facts, lower_to_mir, prepare_vectorization_trial, run_kir_pass_pipeline,
};

const AFFINE_RMW_DIVIDE_SUBTRACT: &str = r#"
export unsafe fn generalized_affine(a: slice<f64>, out: slice<f64>, n: u32) -> void
contract {
  requires n <= a.len && n <= out.len;
  requires noalias(a, out);
  effects read(a), readwrite(out);
}
{
  let i: u32 = 0;
  while i < n {
    let previous: f64 = out[i];
    let source: f64 = a[i];
    let quotient: f64 = previous / source;
    out[i] = quotient - 1.0;
    i = i + 1;
  }
}
"#;

const AFFINE_RMW_BROADCAST_DIVIDE_ADD: &str = r#"
export unsafe fn generalized_broadcast(a: slice<f64>, bias: slice<f64>, out: slice<f64>, n: u32) -> void
contract {
  requires n <= a.len && n <= out.len && bias.len > 0;
  requires noalias(a, out) && noalias(bias, out);
  effects read(a), read(bias), readwrite(out);
}
{
  let i: u32 = 0;
  while i < n {
    let previous: f64 = out[i];
    let source: f64 = a[i];
    let invariant: f64 = bias[0];
    let quotient: f64 = previous / source;
    out[i] = quotient + invariant;
    i = i + 1;
  }
}
"#;

fn wasm_state(source: &str) -> KirVerifiedProgramState {
    let checked = calckernel::check(&SourceFile::new("generalized-vectorization.ck", source));
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
        import_contract_facts(&module, &checked.checked_program, 0).expect("source contract facts");
    let optimized = run_kir_pass_pipeline(module, KirOptimizationLevel::O2, Some(&contracts));
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

fn native_unit_stride_state(source: &str) -> KirVerifiedProgramState {
    let checked = calckernel::check(&SourceFile::new("ordinary-vector-alias.ck", source));
    assert_eq!(checked.diagnostics, [], "invalid fixture: {source}");
    let mir = lower_to_mir(&checked.checked_program).expect("MIR lowering");
    let mut builder = KirTargetProfileBuilder::native(
        KirConsumer::NativeLibrary,
        "x86_64-unknown-linux-gnu",
        64,
        true,
        KirNativeCpuPolicy::Baseline,
        "x86-64",
        vec!["+sse2".to_string()],
    )
    .expect("native profile builder");
    for key in KirTargetProfile::fixed_query_universe()
        .into_iter()
        .filter(|key| {
            key.lane == KirLaneType::U32
                && key.lanes == 4
                && matches!(
                    key.operation,
                    KirProfileOperation::Splat
                        | KirProfileOperation::Add
                        | KirProfileOperation::Subtract
                        | KirProfileOperation::Multiply
                        | KirProfileOperation::Divide
                        | KirProfileOperation::Load
                        | KirProfileOperation::Store
                        | KirProfileOperation::Compare
                        | KirProfileOperation::Select
                        | KirProfileOperation::Cast
                        | KirProfileOperation::Insert
                        | KirProfileOperation::Extract
                        | KirProfileOperation::RuntimePredicate
                )
                && (!matches!(
                    key.operation,
                    KirProfileOperation::Load | KirProfileOperation::Store
                ) || key.alignment == KirAlignmentClass::Bytes(4))
        })
    {
        builder
            .set_legal(
                key,
                KirLegalCost {
                    cost: 1,
                    legalization_parts: 1,
                    legalized_type: "v4i32".to_string(),
                },
            )
            .expect("legal native vector operation");
    }
    let profile = builder.build().expect("native profile");
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
    .expect("native KIR");
    let contracts =
        import_contract_facts(&module, &checked.checked_program, 0).expect("source contract facts");
    let optimized = run_kir_pass_pipeline(module, KirOptimizationLevel::O2, Some(&contracts));
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
fn wasm_affine_rmw_arithmetic_dag_should_interleave_and_check() {
    let state = wasm_state(AFFINE_RMW_DIVIDE_SUBTRACT);
    let discovery = discover_vectorization_candidates(&state);
    let candidate = discovery
        .candidates
        .iter()
        .find(|candidate| candidate.wasm_affine.is_some() && candidate.vf == 2 && candidate.uf == 4)
        .cloned()
        .unwrap_or_else(|| {
            panic!(
                "generic affine RMW candidate missing: {discovery:#?}; source blocks: {:#?}",
                state.module().functions[0].blocks
            )
        });

    let prepared = prepare_vectorization_trial(&state, &candidate).expect("affine RMW trial");
    assert_eq!((prepared.plan.vf, prepared.plan.uf), (2, 4));
    assert_eq!(
        check_vectorization_trial_independently(
            &state,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge,
        ),
        Ok(())
    );
}

#[test]
fn wasm_affine_invariant_broadcast_reuse_should_hold_across_uf_chunks() {
    let state = wasm_state(AFFINE_RMW_BROADCAST_DIVIDE_ADD);
    let discovery = discover_vectorization_candidates(&state);
    let candidate = discovery
        .candidates
        .iter()
        .find(|candidate| candidate.wasm_affine.is_some() && candidate.vf == 2 && candidate.uf == 4)
        .cloned()
        .unwrap_or_else(|| panic!("generic broadcast candidate missing: {discovery:#?}"));

    let prepared = prepare_vectorization_trial(&state, &candidate).expect("broadcast trial");
    assert_eq!(prepared.plan.broadcast_groups.len(), 1);
    assert_eq!(prepared.plan.broadcast_groups[0].unroll_index, 0);
    assert_eq!(
        check_vectorization_trial_independently(
            &state,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge,
        ),
        Ok(())
    );
}

#[test]
fn wasm_affine_unknown_alias_map_should_combine_ranges_with_alias_guard() {
    let source = r#"
export unsafe fn unknown_alias_affine(a: slice<f64>, out: slice<f64>, n: u32) -> void
contract {
  requires n <= a.len && n <= out.len;
  effects read(a), readwrite(out);
}
{
  let i: u32 = 0;
  while i < n {
    let previous: f64 = out[i];
    let input: f64 = a[i];
    out[i] = previous / input - 1.0;
    i = i + 1;
  }
}
"#;
    let state = wasm_state(source);
    let discovery = discover_vectorization_candidates(&state);
    let candidate = discovery
        .candidates
        .iter()
        .find(|candidate| {
            candidate.wasm_affine.is_some()
                && candidate.version_predicate.is_some()
                && candidate.vf == 2
                && candidate.uf == 1
        })
        .cloned()
        .unwrap_or_else(|| {
            panic!("unknown-alias affine candidate with range coverage: {discovery:#?}")
        });
    let prepared = prepare_vectorization_trial(&state, &candidate).expect("combined-guard trial");
    let range_count = prepared
        .plan
        .predicates
        .iter()
        .filter(|predicate| matches!(predicate, VectorPredicate::WasmSliceRange { .. }))
        .count();
    let alias_count = prepared
        .plan
        .predicates
        .iter()
        .filter(|predicate| matches!(predicate, VectorPredicate::AddressNonOverlap { .. }))
        .count();
    assert_eq!(range_count, 2);
    assert_eq!(alias_count, 1);
    assert_eq!(prepared.plan.predicates.len(), 4);
    assert_eq!(
        check_vectorization_trial_independently(
            &state,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge,
        ),
        Ok(())
    );
}

#[test]
fn wasm_affine_store_only_fill_should_keep_its_slice_range_guard() {
    let source = r#"
export unsafe fn fill_affine(out: slice<f64>, n: u32) -> void
contract {
  requires n <= out.len;
  effects write(out);
}
{
  let i: u32 = 0;
  while i < n {
    out[i] = u32_to_f64(i);
    i = i + 1;
  }
}
"#;
    let state = wasm_state(source);
    let discovery = discover_vectorization_candidates(&state);
    let candidate = discovery
        .candidates
        .iter()
        .find(|candidate| candidate.wasm_affine.is_some() && candidate.vf == 2 && candidate.uf == 1)
        .cloned()
        .unwrap_or_else(|| panic!("guarded f64 fill candidate: {discovery:#?}"));
    let prepared = prepare_vectorization_trial(&state, &candidate).expect("guarded fill trial");
    assert_eq!(
        prepared
            .plan
            .predicates
            .iter()
            .filter(|predicate| matches!(predicate, VectorPredicate::WasmSliceRange { .. }))
            .count(),
        1
    );
    assert_eq!(
        check_vectorization_trial_independently(
            &state,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge,
        ),
        Ok(())
    );
}

#[test]
fn wasm_f64_alias_predicate_should_not_replace_memory_range_guard() {
    let source = r#"
export unsafe fn unknown_alias_affine(a: slice<f64>, out: slice<f64>, n: u32) -> void
contract {
  requires n <= a.len && n <= out.len;
  effects read(a), readwrite(out);
}
{
  let i: u32 = 0;
  while i < n {
    let previous: f64 = out[i];
    let input: f64 = a[i];
    out[i] = previous / input - 1.0;
    i = i + 1;
  }
}
"#;
    let state = wasm_state(source);
    let candidate = discover_vectorization_candidates(&state)
        .candidates
        .into_iter()
        .find(|candidate| {
            candidate.wasm_affine.is_some()
                && candidate.version_predicate.is_some()
                && candidate.vf == 2
                && candidate.uf == 1
        })
        .expect("combined guarded affine candidate");
    let prepared = prepare_vectorization_trial(&state, &candidate).expect("combined-guard trial");
    let mut plan = prepared.plan.clone();
    plan.predicates
        .retain(|predicate| !matches!(predicate, VectorPredicate::WasmSliceRange { .. }));
    assert!(
        plan.predicates
            .iter()
            .any(|predicate| { matches!(predicate, VectorPredicate::AddressNonOverlap { .. }) })
    );

    let mut trial = prepared.trial.clone();
    let preheader = trial.module_mut().functions[0]
        .blocks
        .iter_mut()
        .find(|block| block.id == candidate.preheader)
        .expect("candidate preheader");
    let predicate = preheader
        .instructions
        .iter_mut()
        .find_map(|instruction| match &mut instruction.kind {
            calckernel::KirInstructionKind::VersionPredicate { predicate } => Some(predicate),
            _ => None,
        })
        .expect("combined runtime predicate");
    predicate.conjuncts.retain(|conjunct| {
        !matches!(
            conjunct,
            calckernel::KirVersionPredicateConjunct::WasmSliceRange { .. }
        )
    });

    let result = check_vectorization_trial_independently(&state, &trial, &plan, &prepared.charge);
    assert!(
        matches!(
            result,
            Err(calckernel::TransactionCheckError::Compiler(ref message))
                if message == "WASM f64 slice source lacks guarded affine coverage or a checked runtime alias pair"
        ),
        "alias-only guard downgrade was accepted or rejected for the wrong reason: {result:?}"
    );
}

#[test]
fn wasm_affine_checker_should_reject_forged_lane_memory_proof_vf_range_epilogue_and_identity() {
    let state = wasm_state(AFFINE_RMW_BROADCAST_DIVIDE_ADD);
    let candidate = discover_vectorization_candidates(&state)
        .candidates
        .into_iter()
        .find(|candidate| candidate.wasm_affine.is_some() && candidate.vf == 2 && candidate.uf == 4)
        .expect("generic affine candidate");
    let prepared = prepare_vectorization_trial(&state, &candidate).expect("affine trial");
    assert_eq!(
        check_vectorization_trial_independently(
            &state,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge,
        ),
        Ok(())
    );

    let mut lane = prepared.plan.clone();
    lane.operations[0].lanes.swap(0, 1);
    let mut memory = prepared.plan.clone();
    memory.memory_groups[0].vector_instruction = InstructionId::from_index(u32::MAX);
    let mut proof = prepared.plan.clone();
    proof.proofs.operation_equivalence = proof.proofs.canonical_loop;
    let mut vf_uf = prepared.plan.clone();
    vf_uf.uf = 2;
    let mut epilogue = prepared.plan.clone();
    epilogue.epilogue = VectorEpilogue::None;
    let mut stale_identity = prepared.plan.clone();
    stale_identity.pre_state.kir_digest = "11".repeat(32);

    for (dimension, forged) in [
        ("lane map", lane),
        ("memory mapping", memory),
        ("proof root", proof),
        ("VF/UF", vf_uf),
        ("range", range_plan(prepared.plan.clone())),
        ("epilogue", epilogue),
        ("stale identity", stale_identity),
    ] {
        assert!(
            check_vectorization_trial_independently(
                &state,
                &prepared.trial,
                &forged,
                &prepared.charge,
            )
            .is_err(),
            "checker accepted forged {dimension}"
        );
    }

    let mut forged_facts = prepared.trial.clone();
    let function = state
        .module()
        .functions
        .iter()
        .find(|function| function.id == prepared.plan.pre_state.function)
        .expect("source function");
    let source = function.params[0].value;
    let facts = forged_facts
        .contract_facts_mut()
        .expect("trial contract facts")
        .facts_mut();
    let noalias = facts
        .facts()
        .iter()
        .find(|fact| {
            matches!(
                fact.predicate,
                calckernel::FactPredicate::Contract(
                    calckernel::ContractFactPredicate::NoAlias { .. }
                )
            )
        })
        .expect("trial NoAlias fact")
        .id;
    facts
        .get_mut(noalias)
        .expect("mutable trial NoAlias fact")
        .predicate =
        calckernel::FactPredicate::Contract(calckernel::ContractFactPredicate::Aligned {
            pointer: calckernel::ContractFactPointer::SliceData(source),
            alignment: 4096,
        });
    assert!(
        check_vectorization_trial_independently(
            &state,
            &forged_facts,
            &prepared.plan,
            &prepared.charge,
        )
        .is_err(),
        "checker accepted a trial that replaced its trusted NoAlias evidence"
    );

    let mut forged_proof_suffix = prepared.trial.clone();
    let first_new_proof = calckernel::ProofId::from_index(
        u32::try_from(state.proofs().proofs().len()).expect("proof prefix length"),
    );
    forged_proof_suffix
        .proofs_mut()
        .get_mut(first_new_proof)
        .expect("first appended vector proof")
        .id = calckernel::ProofId::from_index(u32::MAX);
    assert!(
        check_vectorization_trial_independently(
            &state,
            &forged_proof_suffix,
            &prepared.plan,
            &prepared.charge,
        )
        .is_err(),
        "checker accepted a vector trial with a forged proof suffix"
    );
}

#[test]
fn wasm_f64_slice_checker_should_require_guards_for_affine_memory() {
    let state = wasm_state(AFFINE_RMW_DIVIDE_SUBTRACT);
    let candidate = discover_vectorization_candidates(&state)
        .candidates
        .into_iter()
        .find(|candidate| candidate.wasm_affine.is_some() && candidate.vf == 2 && candidate.uf == 1)
        .expect("guarded affine VF2/UF1 candidate");
    let prepared = prepare_vectorization_trial(&state, &candidate).expect("affine trial");

    let mut plan = prepared.plan.clone();
    plan.predicates
        .retain(|predicate| matches!(predicate, VectorPredicate::TripThreshold { .. }));
    plan.broadcast_groups.clear();

    let mut trial = prepared.trial.clone();
    let function = trial
        .module_mut()
        .functions
        .iter_mut()
        .find(|function| function.id == candidate.function)
        .expect("trial function");
    let preheader = function
        .blocks
        .iter_mut()
        .find(|block| block.id == candidate.preheader)
        .expect("candidate preheader");
    let predicate = preheader
        .instructions
        .iter_mut()
        .find_map(|instruction| match &mut instruction.kind {
            calckernel::KirInstructionKind::VersionPredicate { predicate } => Some(predicate),
            _ => None,
        })
        .expect("preheader version predicate");
    predicate.conjuncts.retain(|conjunct| {
        matches!(
            conjunct,
            calckernel::KirVersionPredicateConjunct::TripThreshold { .. }
        )
    });

    let result = check_vectorization_trial_independently(&state, &trial, &plan, &prepared.charge);
    assert!(
        matches!(
            result,
            Err(calckernel::TransactionCheckError::Compiler(ref message))
                if message == "WASM f64 slice source lacks guarded affine coverage or a checked runtime alias pair"
        ),
        "checker accepted a range-less ordinary-plan downgrade or rejected it for the wrong reason: {result:?}"
    );
}

fn range_plan(mut plan: calckernel::VectorizationPlan) -> calckernel::VectorizationPlan {
    let range = plan
        .predicates
        .iter_mut()
        .find_map(|predicate| match predicate {
            VectorPredicate::WasmSliceRange { requirement, .. } => Some(requirement),
            _ => None,
        })
        .expect("affine range predicate");
    range.element_bytes = 4;
    plan
}

#[test]
fn wasm_affine_interleave_should_reject_non_induction_loop_state() {
    let source = r#"
export unsafe fn carried_state(a: slice<f64>, out: slice<f64>, n: u32) -> f64
contract {
  requires n <= a.len && n <= out.len;
  requires noalias(a, out);
  effects read(a), readwrite(out);
}

{
  let state: f64 = 0.0;
  let i: u32 = 0;
  while i < n {
    let previous: f64 = out[i];
    let source: f64 = a[i];
    let value: f64 = state + source;
    out[i] = value;
    state = previous;
    i = i + 1;
  }
  return state;
    }
"#;
    let state = wasm_state(source);
    let discovery = discover_vectorization_candidates(&state);
    assert!(
        discovery
            .candidates
            .iter()
            .all(|candidate| candidate.wasm_affine.is_none() || candidate.uf == 1)
    );
    assert!(
        discovery.fallbacks.iter().any(|fallback| {
            fallback.reason == "wasm-affine-loop-has-non-induction-carried-state"
        }),
        "the loop-carried state should be the recorded legality reason: {discovery:#?}"
    );
}

#[test]
fn wasm_affine_checker_should_reject_changed_non_induction_backedge_state() {
    use calckernel::{
        KirBlockParam, KirInstructionKind as Kind, KirTerminator as Term, KirValueType,
        MirPrimitiveTypeName, MirType, ValueId,
    };

    let mut state = wasm_state(AFFINE_RMW_DIVIDE_SUBTRACT);
    let candidate = discover_vectorization_candidates(&state)
        .candidates
        .into_iter()
        .find(|candidate| candidate.wasm_affine.is_some() && candidate.vf == 2 && candidate.uf == 1)
        .expect("affine VF2/UF1 candidate");
    let mut prepared = prepare_vectorization_trial(&state, &candidate).expect("affine trial");
    assert_eq!(
        check_vectorization_trial_independently(
            &state,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge,
        ),
        Ok(())
    );

    let next_value = prepared.trial.module().functions[0]
        .blocks
        .iter()
        .flat_map(|block| {
            block.params.iter().map(|param| param.value.index()).chain(
                block.instructions.iter().flat_map(|instruction| {
                    instruction
                        .results
                        .iter()
                        .map(|result| result.value.index())
                }),
            )
        })
        .chain(
            prepared.trial.module().functions[0]
                .params
                .iter()
                .map(|param| param.value.index()),
        )
        .max()
        .expect("value IDs")
        + 1;
    let value = |offset| ValueId::from_index(next_value + offset);
    let next_instruction = prepared.trial.module().functions[0]
        .blocks
        .iter()
        .flat_map(|block| {
            block
                .instructions
                .iter()
                .map(|instruction| instruction.id.index())
        })
        .max()
        .expect("instruction IDs")
        + 1;
    let scalar = KirValueType::Scalar(MirType::Primitive(MirPrimitiveTypeName::F64));
    let scalar_one = state.module().functions[0]
        .blocks
        .iter()
        .find(|block| block.id == candidate.body)
        .expect("source body")
        .instructions
        .iter()
        .find(
            |instruction| matches!(&instruction.kind, Kind::ConstFloat { value } if value == "1.0"),
        )
        .expect("scalar one")
        .results[0]
        .value;
    let preheader_prefix = state.module().functions[0]
        .blocks
        .iter()
        .find(|block| block.id == candidate.preheader)
        .expect("source preheader")
        .instructions
        .len();
    let subtraction = candidate
        .operations
        .iter()
        .find(|operation| operation.operation == KirProfileOperation::Subtract)
        .expect("subtract operation")
        .scalar;
    let zero = calckernel::KirInstruction {
        id: InstructionId::from_index(next_instruction),
        results: vec![calckernel::KirResult {
            value: value(4),
            type_node: scalar.clone(),
        }],
        kind: Kind::ConstFloat {
            value: "0.0".to_string(),
        },
        memory: None,
        effect: None,
    };

    for (module, is_trial) in [
        (state.module_mut(), false),
        (prepared.trial.module_mut(), true),
    ] {
        let function = &mut module.functions[0];
        let preheader = function
            .blocks
            .iter_mut()
            .find(|block| block.id == candidate.preheader)
            .expect("preheader");
        preheader
            .instructions
            .insert(preheader_prefix, zero.clone());
        match &mut preheader.terminator {
            Term::Jump { edge } => edge.args.push(value(4)),
            Term::Branch {
                then_edge,
                else_edge,
                ..
            } => {
                then_edge.args.push(value(4));
                else_edge.args.push(value(4));
            }
            _ => panic!("preheader terminator"),
        }

        let header = function
            .blocks
            .iter_mut()
            .find(|block| block.id == candidate.header)
            .expect("source header");
        header.params.push(KirBlockParam {
            value: value(0),
            slot: "carried".to_string(),
            type_node: scalar.clone(),
        });
        if let Term::Branch { then_edge, .. } = &mut header.terminator {
            then_edge.args.push(value(0));
        }

        let body = function
            .blocks
            .iter_mut()
            .find(|block| block.id == candidate.body)
            .expect("source body");
        body.params.push(KirBlockParam {
            value: value(1),
            slot: "carried".to_string(),
            type_node: scalar.clone(),
        });
        if let Kind::Binary { right, .. } = &mut body
            .instructions
            .iter_mut()
            .find(|instruction| instruction.id == subtraction)
            .expect("source subtraction")
            .kind
        {
            *right = value(1);
        }
        if let Term::Jump { edge } = &mut body.terminator {
            edge.args.push(scalar_one);
        }

        if is_trial {
            let vector_header = function
                .blocks
                .iter_mut()
                .find(|block| block.label == "loop_simd_header")
                .expect("vector header");
            vector_header.params.push(KirBlockParam {
                value: value(2),
                slot: "carried".to_string(),
                type_node: scalar.clone(),
            });
            if let Term::Branch {
                then_edge,
                else_edge,
                ..
            } = &mut vector_header.terminator
            {
                then_edge.args.push(value(2));
                else_edge.args.push(value(2));
            }

            let vector_body = function
                .blocks
                .iter_mut()
                .find(|block| block.label == "loop_simd_body")
                .expect("vector body");
            vector_body.params.push(KirBlockParam {
                value: value(3),
                slot: "carried".to_string(),
                type_node: scalar.clone(),
            });
            let vector_one = vector_body
                .instructions
                .iter()
                .find(|instruction| {
                    matches!(&instruction.kind, Kind::ConstFloat { value } if value == "1.0")
                })
                .expect("vector one")
                .results[0]
                .value;
            for instruction in &mut vector_body.instructions {
                if let Kind::VectorSplat { scalar, .. } = &mut instruction.kind
                    && *scalar == vector_one
                {
                    *scalar = value(3);
                }
            }
            if let Term::Jump { edge } = &mut vector_body.terminator {
                edge.args.push(vector_one);
            }
        }
    }

    prepared.plan.pre_state.kir_digest = state.kir_digest();
    let before = calckernel::kir_function_units(&state.module().functions[0]);
    prepared.plan.pre_state.frozen_kir_units = before;
    let after = calckernel::kir_function_units(&prepared.trial.module().functions[0]);
    prepared.plan.growth = calckernel::VectorPlanGrowth::new(before, after, before, after);
    assert!(
        calckernel::validate_kir_module(state.module())
            .errors
            .is_empty()
    );
    assert!(
        calckernel::validate_kir_module(prepared.trial.module())
            .errors
            .is_empty()
    );
    assert!(
        KirVerifiedProgramState::from_parts(
            prepared.trial.module().clone(),
            prepared.trial.contract_facts().cloned(),
            prepared.trial.proofs().clone(),
            prepared.trial.eliminated_guards().to_vec(),
            prepared.trial.evidence_generation(),
        )
        .is_ok()
    );

    let result = check_vectorization_trial_independently(
        &state,
        &prepared.trial,
        &prepared.plan,
        &prepared.charge,
    );
    assert!(
        matches!(
            result,
            Err(calckernel::TransactionCheckError::Compiler(ref message))
                if message == "WASM loop changes unsupported non-induction carried scalar state"
        ),
        "checker accepted a changed scalar backedge or failed for the wrong reason: {result:?}"
    );
}

#[test]
fn ordinary_vector_checker_requires_noalias_at_the_candidate_preheader() {
    let source = r#"
export unsafe fn ordinary_alias_map(a: slice<u32>, out: slice<u32>, n: u32) -> void
contract {
  requires n <= a.len && n <= out.len;
  requires noalias(a, out);
  effects read(a), write(out);
}
{
  let i: u32 = 0;
  while i < n {
    out[i] = a[i] + 7;
    i = i + 1;
  }
}
"#;
    let mut state = native_unit_stride_state(source);
    let candidate = discover_vectorization_candidates(&state)
        .candidates
        .into_iter()
        .find(|candidate| candidate.wasm_affine.is_none() && candidate.vf == 4)
        .expect("ordinary unit-stride candidate");
    let mut prepared = prepare_vectorization_trial(&state, &candidate).expect("vector trial");
    assert_eq!(
        check_vectorization_trial_independently(
            &state,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge,
        ),
        Ok(())
    );

    let function = state
        .module()
        .functions
        .iter()
        .find(|function| function.id == prepared.plan.pre_state.function)
        .expect("source function");
    let exit = function
        .blocks
        .iter()
        .find(|block| matches!(block.terminator, calckernel::KirTerminator::Return { .. }))
        .expect("exit block")
        .id;
    let function_id = function.id;
    let noalias = state
        .contract_facts()
        .expect("contract facts")
        .facts()
        .facts()
        .iter()
        .find(|fact| {
            matches!(
                fact.predicate,
                calckernel::FactPredicate::Contract(
                    calckernel::ContractFactPredicate::NoAlias { .. }
                )
            )
        })
        .expect("NoAlias contract")
        .id;
    for state in [&mut state, &mut prepared.trial] {
        state
            .contract_facts_mut()
            .expect("contract facts")
            .facts_mut()
            .get_mut(noalias)
            .expect("NoAlias fact")
            .scope = calckernel::FactScope::Block {
            function: function_id,
            block: exit,
        };
    }

    let result = check_vectorization_trial_independently(
        &state,
        &prepared.trial,
        &prepared.plan,
        &prepared.charge,
    );
    assert!(
        matches!(
            result,
            Err(calckernel::TransactionCheckError::Compiler(ref message))
                if message.contains("vector runtime noalias predicates do not exactly close dependence legality")
        ),
        "checker rejected for the wrong reason or accepted unavailable NoAlias evidence: {result:?}"
    );
}

#[test]
fn ordinary_vector_checker_should_reject_unrooted_selected_write_descriptor() {
    use calckernel::{
        BlockId, KirBlock, KirBlockParam, KirEdge, KirInstruction, KirInstructionKind as Kind,
        KirParam, KirResult, KirTerminator as Term, KirValueType, MirBinaryOp,
        MirPrimitiveTypeName, MirType, ValueId,
    };

    let source = r#"
export unsafe fn ordinary_alias_map(a: slice<u32>, out: slice<u32>, n: u32) -> void
contract {
  requires n <= a.len && n <= out.len;
  requires noalias(a, out);
  effects read(a), write(out);
}
{
  let i: u32 = 0;
  while i < n {
    out[i] = a[i] + 7;
    i = i + 1;
  }
}
"#;
    let mut state = native_unit_stride_state(source);
    let candidate = discover_vectorization_candidates(&state)
        .candidates
        .into_iter()
        .find(|candidate| candidate.wasm_affine.is_none() && candidate.vf == 4)
        .expect("ordinary unit-stride candidate");
    let mut prepared = prepare_vectorization_trial(&state, &candidate).expect("vector trial");
    assert_eq!(
        check_vectorization_trial_independently(
            &state,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge,
        ),
        Ok(())
    );

    let next_value = prepared.trial.module().functions[0]
        .blocks
        .iter()
        .flat_map(|block| {
            block.params.iter().map(|param| param.value.index()).chain(
                block.instructions.iter().flat_map(|instruction| {
                    instruction
                        .results
                        .iter()
                        .map(|result| result.value.index())
                }),
            )
        })
        .chain(
            state.module().functions[0]
                .params
                .iter()
                .map(|param| param.value.index()),
        )
        .max()
        .expect("value IDs")
        + 1;
    let next_instruction = prepared.trial.module().functions[0]
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .map(|instruction| instruction.id.index())
        .max()
        .expect("instruction IDs")
        + 1;
    let next_block = prepared.trial.module().functions[0]
        .blocks
        .iter()
        .map(|block| block.id.index())
        .max()
        .expect("block IDs")
        + 1;
    let value = |offset| ValueId::from_index(next_value + offset);
    let block_id = |offset| BlockId::from_index(next_block + offset);
    let slice_type = MirType::Slice(Box::new(MirType::Primitive(MirPrimitiveTypeName::U32)));

    for evidence in [&mut state, &mut prepared.trial] {
        let function = &mut evidence.module_mut().functions[0];
        let input = function.params[0].value;
        let output = function.params[1].value;
        let count = function.params[2].value;
        function.params.push(KirParam {
            value: value(0),
            name: "choose_alternate".to_string(),
            type_node: MirType::Primitive(MirPrimitiveTypeName::Bool),
        });

        let preheader = function
            .blocks
            .iter_mut()
            .find(|block| block.id == candidate.preheader)
            .expect("preheader");
        preheader.params.push(KirBlockParam {
            value: value(1),
            slot: "selected_output".to_string(),
            type_node: KirValueType::Scalar(slice_type.clone()),
        });
        match &mut preheader.terminator {
            Term::Jump { edge } => {
                for argument in &mut edge.args {
                    if *argument == output {
                        *argument = value(1);
                    }
                }
            }
            Term::Branch {
                then_edge,
                else_edge,
                ..
            } => {
                for edge in [then_edge, else_edge] {
                    for argument in &mut edge.args {
                        if *argument == output {
                            *argument = value(1);
                        }
                    }
                }
            }
            _ => panic!("preheader terminator"),
        }

        let edge = |target, args| KirEdge {
            target,
            args,
            memory_args: Vec::new(),
        };
        let block = |id, instructions, terminator| KirBlock {
            id,
            label: "descriptor_select".to_string(),
            params: Vec::new(),
            memory_params: Vec::new(),
            instructions,
            terminator,
        };
        let alternate = block(
            block_id(1),
            vec![
                KirInstruction {
                    id: InstructionId::from_index(next_instruction),
                    results: vec![KirResult {
                        value: value(2),
                        type_node: KirValueType::Scalar(MirType::Primitive(
                            MirPrimitiveTypeName::U32,
                        )),
                    }],
                    kind: Kind::ConstInt {
                        value: "1".to_string(),
                    },
                    memory: None,
                    effect: None,
                },
                KirInstruction {
                    id: InstructionId::from_index(next_instruction + 1),
                    results: vec![KirResult {
                        value: value(4),
                        type_node: KirValueType::Scalar(MirType::Primitive(
                            MirPrimitiveTypeName::U32,
                        )),
                    }],
                    kind: Kind::Binary {
                        op: MirBinaryOp::Add,
                        left: count,
                        right: value(2),
                        semantics: calckernel::KirArithmeticSemantics::Modular,
                    },
                    memory: None,
                    effect: None,
                },
                KirInstruction {
                    id: InstructionId::from_index(next_instruction + 2),
                    results: vec![KirResult {
                        value: value(3),
                        type_node: KirValueType::Scalar(slice_type.clone()),
                    }],
                    kind: Kind::Subslice {
                        slice: input,
                        start: value(2),
                        end: value(4),
                    },
                    memory: None,
                    effect: None,
                },
            ],
            Term::Jump {
                edge: edge(candidate.preheader, vec![value(3)]),
            },
        );
        let passthrough = block(
            block_id(2),
            Vec::new(),
            Term::Jump {
                edge: edge(candidate.preheader, vec![output]),
            },
        );
        let entry = block(
            block_id(0),
            Vec::new(),
            Term::Branch {
                condition: value(0),
                then_edge: edge(block_id(1), Vec::new()),
                else_edge: edge(block_id(2), Vec::new()),
            },
        );
        function.blocks.insert(0, entry);
        function.blocks.push(alternate);
        function.blocks.push(passthrough);
    }

    prepared.plan.pre_state.kir_digest = state.kir_digest();
    let before = calckernel::kir_function_units(&state.module().functions[0]);
    let after = calckernel::kir_function_units(&prepared.trial.module().functions[0]);
    prepared.plan.pre_state.frozen_kir_units = before;
    prepared.plan.growth = calckernel::VectorPlanGrowth::new(before, after, before, after);
    assert!(
        calckernel::validate_kir_module(state.module())
            .errors
            .is_empty()
    );
    assert!(
        calckernel::validate_kir_module(prepared.trial.module())
            .errors
            .is_empty()
    );
    assert!(
        KirVerifiedProgramState::from_parts(
            state.module().clone(),
            state.contract_facts().cloned(),
            state.proofs().clone(),
            state.eliminated_guards().to_vec(),
            state.evidence_generation(),
        )
        .is_ok()
    );
    assert!(
        KirVerifiedProgramState::from_parts(
            prepared.trial.module().clone(),
            prepared.trial.contract_facts().cloned(),
            prepared.trial.proofs().clone(),
            prepared.trial.eliminated_guards().to_vec(),
            prepared.trial.evidence_generation(),
        )
        .is_ok()
    );
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
