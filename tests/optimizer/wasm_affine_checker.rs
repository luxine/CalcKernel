use calckernel::{
    KirBoundsMode, KirBuildConfig, KirOptimizationLevel, KirOverflowMode, KirSanitizerMode,
    KirTargetProfile, KirVerifiedProgramState, KirWasmFeatures, SourceFile, VectorBroadcastGroup,
    build_kir_module_with_profile, check, check_vectorization_trial_independently,
    discover_vectorization_candidates, import_contract_facts, lower_to_mir,
    prepare_vectorization_trial, run_kir_pass_pipeline,
};

fn wasm_state(source: &str) -> KirVerifiedProgramState {
    wasm_state_at_level(source, KirOptimizationLevel::O2)
}

fn wasm_state_at_level(source: &str, level: KirOptimizationLevel) -> KirVerifiedProgramState {
    let checked = check(&SourceFile::new("affine-checker.ck", source));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR");
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
    .expect("KIR");
    let contracts = import_contract_facts(&module, &checked.checked_program, 0).expect("contracts");
    let optimized = run_kir_pass_pipeline(module, level, Some(&contracts));
    assert!(optimized.errors.is_empty(), "{:?}", optimized.errors);
    KirVerifiedProgramState::from_parts(
        optimized.artifact.expect("optimized KIR"),
        optimized.contract_facts,
        optimized.proofs,
        optimized.eliminated_guards,
        0,
    )
    .expect("verified state")
}

const MAP: &str = r#"
export unsafe fn map(a: slice<f64>, b: slice<f64>, n: u32) -> void
contract { requires noalias(a, b); effects read(a), write(b); }
{
  let i: u32 = 0;
  while i < n { b[i] = a[i] + 7.0; i = i + 1; }
}
"#;

#[test]
fn checker_rejects_unaccounted_broadcast_group_on_ordinary_map() {
    let state = wasm_state(MAP);
    let candidate = discover_vectorization_candidates(&state)
        .candidates
        .into_iter()
        .next()
        .expect("map candidate");
    let mut prepared = prepare_vectorization_trial(&state, &candidate).expect("map trial");
    assert!(
        check_vectorization_trial_independently(
            &state,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge,
        )
        .is_ok()
    );
    let memory = &prepared.plan.memory_groups[0];
    prepared.plan.broadcast_groups.push(VectorBroadcastGroup {
        region: memory.region,
        scalar_instruction: memory.scalar_instructions[0],
        emitted_scalar_load: memory.vector_instruction,
        emitted_splat: prepared.plan.operations[0].vector,
        unroll_index: 0,
        footprint_proof: memory.footprint_proof,
    });
    assert!(
        check_vectorization_trial_independently(
            &state,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge,
        )
        .is_err(),
        "unaccounted broadcast records must never be silently accepted"
    );
}

const AFFINE: &str = r#"
export unsafe fn affine(a: slice<f64>, b: slice<f64>, out: slice<f64>, n: u32, offset: u32, column: u32) -> void
contract {
  requires offset + n <= b.len && offset + n <= out.len && column < a.len;
  requires noalias(a, out) && noalias(b, out);
  effects read(a), read(b), readwrite(out);
}
{
  let i: u32 = 0;
  while i < n {
    let index: u32 = offset + i;
    let previous: f64 = out[index];
    let scalar: f64 = a[column];
    let varying: f64 = b[index];
    out[index] = previous + scalar * varying;
    i = i + 1;
  }
}
"#;

fn affine_trial() -> (KirVerifiedProgramState, calckernel::PreparedVectorization) {
    let state = wasm_state(AFFINE);
    let candidate = discover_vectorization_candidates(&state)
        .candidates
        .into_iter()
        .find(|candidate| candidate.wasm_affine.is_some())
        .expect("affine candidate");
    let prepared = prepare_vectorization_trial(&state, &candidate).expect("affine trial");
    let checked = check_vectorization_trial_independently(
        &state,
        &prepared.trial,
        &prepared.plan,
        &prepared.charge,
    );
    assert!(
        checked.is_ok(),
        "genuine affine trial must pass independent checks: {checked:?}"
    );
    (state, prepared)
}

#[test]
fn checker_accepts_strict_affine_trial_with_exact_broadcast_and_ranges() {
    let _ = affine_trial();
}

#[test]
fn checker_rejects_forged_affine_range_counts_and_missing_ranges() {
    for missing in [false, true] {
        let (state, mut prepared) = affine_trial();
        let position = prepared
            .plan
            .predicates
            .iter()
            .position(|predicate| {
                matches!(predicate,
            calckernel::VectorPredicate::WasmSliceRange { requirement, .. }
            if matches!(requirement.count, calckernel::WasmRangeCount::TripBound(_)))
            })
            .expect("trip footprint");
        if missing {
            prepared.plan.predicates.remove(position);
        } else if let calckernel::VectorPredicate::WasmSliceRange { requirement, .. } =
            &mut prepared.plan.predicates[position]
        {
            requirement.count = calckernel::WasmRangeCount::One;
        }
        assert!(
            check_vectorization_trial_independently(
                &state,
                &prepared.trial,
                &prepared.plan,
                &prepared.charge
            )
            .is_err()
        );
    }
}

#[test]
fn checker_rejects_duplicate_or_rebound_affine_broadcast_groups() {
    for duplicate in [false, true] {
        let (state, mut prepared) = affine_trial();
        if duplicate {
            prepared
                .plan
                .broadcast_groups
                .push(prepared.plan.broadcast_groups[0].clone());
        } else {
            prepared.plan.broadcast_groups[0].emitted_splat = prepared.plan.operations[0].vector;
        }
        assert!(
            check_vectorization_trial_independently(
                &state,
                &prepared.trial,
                &prepared.plan,
                &prepared.charge
            )
            .is_err()
        );
    }
}

#[test]
fn checker_rejects_changed_affine_start_and_exclusive_end() {
    for change_start in [false, true] {
        let (state, mut prepared) = affine_trial();
        let id = prepared.plan.memory_groups[0].vector_instruction;
        let function = prepared
            .trial
            .module_mut()
            .functions
            .iter_mut()
            .find(|function| function.id == prepared.plan.pre_state.function)
            .expect("function");
        let bound = function
            .params
            .iter()
            .find(|param| param.name == "n")
            .expect("bound")
            .value;
        let instruction = function
            .blocks
            .iter_mut()
            .flat_map(|block| &mut block.instructions)
            .find(|instruction| instruction.id == id)
            .expect("vector load");
        let access = match &mut instruction.kind {
            calckernel::KirInstructionKind::VectorLoad { access, .. }
            | calckernel::KirInstructionKind::VectorStore { access, .. } => access,
            _ => panic!("vector access"),
        };
        if change_start {
            access.start = bound;
        } else {
            access.end = bound;
        }
        assert!(
            check_vectorization_trial_independently(
                &state,
                &prepared.trial,
                &prepared.plan,
                &prepared.charge
            )
            .is_err()
        );
    }
}

#[test]
fn checker_rejects_changed_affine_broadcast_address() {
    let (state, mut prepared) = affine_trial();
    let id = prepared.plan.broadcast_groups[0].emitted_scalar_load;
    let function = prepared
        .trial
        .module_mut()
        .functions
        .iter_mut()
        .find(|function| function.id == prepared.plan.pre_state.function)
        .expect("function");
    let offset = function
        .params
        .iter()
        .find(|param| param.name == "offset")
        .expect("offset")
        .value;
    let instruction = function
        .blocks
        .iter_mut()
        .flat_map(|block| &mut block.instructions)
        .find(|instruction| instruction.id == id)
        .expect("broadcast load");
    if let calckernel::KirInstructionKind::Load { place } = &mut instruction.kind
        && let calckernel::KirPlace::SliceIndex { index, .. } = place.as_mut()
    {
        *index = offset;
    }
    assert!(
        check_vectorization_trial_independently(
            &state,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge
        )
        .is_err()
    );
}

#[test]
fn checker_rejects_changed_strict_affine_arithmetic_dataflow() {
    let (state, mut prepared) = affine_trial();
    let id = prepared.plan.operations[0].vector;
    let instruction = prepared
        .trial
        .module_mut()
        .functions
        .iter_mut()
        .flat_map(|function| &mut function.blocks)
        .flat_map(|block| &mut block.instructions)
        .find(|instruction| instruction.id == id)
        .expect("arithmetic");
    if let calckernel::KirInstructionKind::VectorBinary { left, right, .. } = &mut instruction.kind
    {
        *right = *left;
    }
    assert!(
        check_vectorization_trial_independently(
            &state,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge
        )
        .is_err()
    );
}

#[test]
fn checker_rejects_reordered_affine_reads_and_broadcast_splat() {
    let (state, mut prepared) = affine_trial();
    let id = prepared.plan.broadcast_groups[0].emitted_splat;
    let block = prepared
        .trial
        .module_mut()
        .functions
        .iter_mut()
        .flat_map(|function| &mut function.blocks)
        .find(|block| {
            block
                .instructions
                .iter()
                .any(|instruction| instruction.id == id)
        })
        .expect("vector body");
    let index = block
        .instructions
        .iter()
        .position(|instruction| instruction.id == id)
        .expect("splat");
    block.instructions.swap(index, index + 1);
    assert!(
        check_vectorization_trial_independently(
            &state,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge
        )
        .is_err()
    );
}

#[test]
fn checker_rejects_changed_affine_memory_input() {
    let (state, mut prepared) = affine_trial();
    let load = prepared.plan.broadcast_groups[0].emitted_scalar_load;
    let function = prepared
        .trial
        .module_mut()
        .functions
        .iter_mut()
        .find(|function| function.id == prepared.plan.pre_state.function)
        .expect("function");
    let original_input = function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .find(|instruction| instruction.id == load)
        .expect("load")
        .memory
        .as_ref()
        .expect("MemorySSA")
        .input;
    let other = function
        .blocks
        .iter()
        .find(|block| block.label == "loop_simd_header")
        .expect("header")
        .memory_params
        .iter()
        .find(|param| param.version != original_input)
        .expect("different memory version")
        .version;
    assert_ne!(
        original_input, other,
        "the adversarial edit must change MemorySSA"
    );
    function
        .blocks
        .iter_mut()
        .flat_map(|block| &mut block.instructions)
        .find(|instruction| instruction.id == load)
        .expect("load")
        .memory
        .as_mut()
        .expect("MemorySSA")
        .input = other;
    assert!(
        check_vectorization_trial_independently(
            &state,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge
        )
        .is_err()
    );
}

#[test]
fn checker_rejects_forged_affine_predicate_and_scalar_tail() {
    for change_tail in [false, true] {
        let (state, mut prepared) = affine_trial();
        let function = prepared
            .trial
            .module_mut()
            .functions
            .iter_mut()
            .find(|function| function.id == prepared.plan.pre_state.function)
            .expect("function");
        if change_tail {
            let header = function
                .blocks
                .iter_mut()
                .find(|block| block.label == "loop_simd_header")
                .expect("header");
            if let calckernel::KirTerminator::Branch { else_edge, .. } = &mut header.terminator {
                else_edge.args.swap(0, 1);
            }
        } else {
            let count = function
                .params
                .iter()
                .find(|param| param.name == "offset")
                .expect("offset")
                .value;
            let predicate = function
                .blocks
                .iter_mut()
                .flat_map(|block| &mut block.instructions)
                .find_map(|instruction| match &mut instruction.kind {
                    calckernel::KirInstructionKind::VersionPredicate { predicate } => {
                        Some(predicate)
                    }
                    _ => None,
                })
                .expect("predicate");
            let range = predicate
                .conjuncts
                .iter_mut()
                .find(|conjunct| {
                    matches!(
                        conjunct,
                        calckernel::KirVersionPredicateConjunct::WasmSliceRange { .. }
                    )
                })
                .expect("range");
            if let calckernel::KirVersionPredicateConjunct::WasmSliceRange {
                count: actual, ..
            } = range
            {
                *actual = count;
            }
        }
        assert!(
            check_vectorization_trial_independently(
                &state,
                &prepared.trial,
                &prepared.plan,
                &prepared.charge
            )
            .is_err()
        );
    }
}

#[test]
fn checker_rejects_noalias_evidence_from_a_nondominating_block() {
    let (mut state, prepared) = affine_trial();
    let function = state
        .module()
        .functions
        .iter()
        .find(|function| function.id == prepared.plan.pre_state.function)
        .expect("function");
    let exit = function
        .blocks
        .iter()
        .find(|block| matches!(block.terminator, calckernel::KirTerminator::Return { .. }))
        .expect("exit")
        .id;
    let function_id = function.id;
    let facts = state.contract_facts_mut().expect("facts").facts_mut();
    let id = facts
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
        .expect("noalias")
        .id;
    facts.get_mut(id).expect("fact").scope = calckernel::FactScope::Block {
        function: function_id,
        block: exit,
    };
    assert!(
        check_vectorization_trial_independently(
            &state,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge
        )
        .is_err()
    );
}

#[test]
fn checker_rejects_changed_affine_scalar_address_setup() {
    let (state, mut prepared) = affine_trial();
    let function = prepared
        .trial
        .module_mut()
        .functions
        .iter_mut()
        .find(|function| function.id == prepared.plan.pre_state.function)
        .expect("function");
    let column = function
        .params
        .iter()
        .find(|param| param.name == "column")
        .expect("column")
        .value;
    let body = function
        .blocks
        .iter_mut()
        .find(|block| block.label == "loop_simd_body")
        .expect("body");
    let setup = body
        .instructions
        .iter_mut()
        .find(|instruction| {
            matches!(
                instruction.kind,
                calckernel::KirInstructionKind::Binary { .. }
            )
        })
        .expect("setup");
    if let calckernel::KirInstructionKind::Binary { right, .. } = &mut setup.kind {
        *right = column;
    }
    assert!(
        check_vectorization_trial_independently(
            &state,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge
        )
        .is_err()
    );
}

#[test]
fn checker_rejects_underpriced_affine_broadcast_and_address_setup() {
    let (state, mut prepared) = affine_trial();
    prepared.plan.cost.transformed_body = prepared.plan.cost.transformed_body.saturating_sub(1);
    assert!(
        check_vectorization_trial_independently(
            &state,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge
        )
        .is_err()
    );
}

fn ordinary_trial(source: &str) -> (KirVerifiedProgramState, calckernel::PreparedVectorization) {
    let state = wasm_state(source);
    let candidate = discover_vectorization_candidates(&state)
        .candidates
        .into_iter()
        .next()
        .expect("map candidate");
    let prepared = prepare_vectorization_trial(&state, &candidate).expect("map trial");
    let result = check_vectorization_trial_independently(
        &state,
        &prepared.trial,
        &prepared.plan,
        &prepared.charge,
    );
    assert!(result.is_ok(), "genuine map trial: {result:?}");
    (state, prepared)
}

#[test]
fn checker_rejects_target_abi_export_and_same_type_parameter_mutations() {
    for mutation in 0..6 {
        let (state, mut prepared) = ordinary_trial(MAP);
        let function = prepared
            .trial
            .module_mut()
            .functions
            .iter_mut()
            .find(|function| function.id == prepared.plan.pre_state.function)
            .expect("function");
        match mutation {
            0 => function.exported = false,
            1 => {
                assert_eq!(function.params[0].type_node, function.params[1].type_node);
                function.params.swap(0, 1);
            }
            2 => function.name = "forged_name".to_string(),
            3 => {
                function.return_type =
                    calckernel::MirType::Primitive(calckernel::MirPrimitiveTypeName::U32)
            }
            4 => function.regions.swap(0, 1),
            5 => function.initial_memory.swap(0, 1),
            _ => unreachable!(),
        }
        let result = check_vectorization_trial_independently(
            &state,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge,
        );
        let error = result.expect_err("forged target ABI or memory metadata was accepted");
        assert!(
            format!("{error:?}").contains("target function ABI or memory metadata"),
            "mutation {mutation}: {error:?}"
        );
    }
}

#[test]
fn checker_rejects_preheader_same_type_parameter_permutation() {
    let source = r#"
export unsafe fn map(a: slice<f64>, b: slice<f64>, n: u32, m: u32, choose: bool) -> void
contract { requires noalias(a,b); effects read(a), write(b); }
{
  let seed: f64 = 1.0;
  if choose { seed = 2.0; }
  let count: u32 = n + 1;
  let i: u32 = 0;
  while i < count { b[i] = a[i] + seed; i = i + 1; }
}
"#;
    for mutation in 0..3 {
        let (state, mut prepared) = ordinary_trial(source);
        let before = state
            .module()
            .functions
            .iter()
            .find(|function| function.id == prepared.plan.pre_state.function)
            .expect("source");
        let function = prepared
            .trial
            .module_mut()
            .functions
            .iter_mut()
            .find(|function| function.id == prepared.plan.pre_state.function)
            .expect("function");
        let preheader = function
            .blocks
            .iter_mut()
            .find(|block| {
                before
                    .blocks
                    .iter()
                    .any(|source| source.id == block.id && source != *block)
            })
            .expect("changed preheader");
        let pair = preheader
            .params
            .iter()
            .enumerate()
            .find_map(|(left, first)| {
                preheader
                    .params
                    .iter()
                    .enumerate()
                    .skip(left + 1)
                    .find(|(_, second)| second.type_node == first.type_node)
                    .map(|(right, _)| (left, right))
            })
            .expect("preheader must have same-type parameters");
        match mutation {
            0 => preheader.params.swap(pair.0, pair.1),
            1 => preheader.memory_params.swap(0, 1),
            2 => preheader.label = "forged_preheader".to_string(),
            _ => unreachable!(),
        }
        let result = check_vectorization_trial_independently(
            &state,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge,
        );
        let error =
            result.expect_err("forged preheader parameter permutation or metadata was accepted");
        assert!(
            format!("{error:?}").contains("preheader parameters or identity"),
            "mutation {mutation}: {error:?}"
        );
    }
}

#[test]
fn checker_rejects_module_configuration_and_identity_mutations() {
    for mutation in 0..5 {
        let (state, mut prepared) = ordinary_trial(MAP);
        let module = prepared.trial.module_mut();
        match mutation {
            0 => module.config.bounds_mode = KirBoundsMode::Checked,
            1 => {
                module.profile =
                    KirTargetProfile::webassembly_with_features(KirWasmFeatures::Baseline)
            }
            2 => {
                module.entry = Some(calckernel::MirEntryPoint {
                    function_name: "map".to_string(),
                    result: calckernel::MirEntryResult::Void,
                })
            }
            3 => module.structs.push(calckernel::MirStruct {
                name: "ForgedStruct".to_string(),
                fields: vec![],
            }),
            4 => module.functions.push(module.functions[0].clone()),
            _ => unreachable!(),
        }
        let error = check_vectorization_trial_independently(
            &state,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge,
        )
        .expect_err("forged module identity was accepted");
        assert!(
            format!("{error:?}").contains("module configuration or identity"),
            "mutation {mutation}: {error:?}"
        );
    }
}

#[test]
fn checker_rejects_extra_or_changed_vector_region_ownership() {
    for extra in [false, true] {
        let (state, mut prepared) = ordinary_trial(MAP);
        let function = prepared
            .trial
            .module_mut()
            .functions
            .iter_mut()
            .find(|function| function.id == prepared.plan.pre_state.function)
            .expect("function");
        if extra {
            function
                .vector_regions
                .push(function.vector_regions[0].clone());
        } else {
            function.vector_regions[0]
                .blocks
                .push(function.blocks[0].id);
        }
        let error = check_vectorization_trial_independently(
            &state,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge,
        )
        .expect_err("forged vector ownership was accepted");
        assert!(format!("{error:?}").contains("vector region"), "{error:?}");
    }
}

#[test]
fn checker_preserves_preexisting_vector_region_metadata() {
    for remove in [false, true] {
        let (mut state, mut prepared) = ordinary_trial(MAP);
        let before = state
            .module_mut()
            .functions
            .iter_mut()
            .find(|function| function.id == prepared.plan.pre_state.function)
            .expect("source");
        let exit = before
            .blocks
            .iter()
            .find(|block| matches!(block.terminator, calckernel::KirTerminator::Return { .. }))
            .expect("exit")
            .id;
        let retained = calckernel::KirVectorRegion {
            id: calckernel::VectorRegionId::from_index(1000),
            blocks: vec![exit],
        };
        before.vector_regions.push(retained.clone());
        prepared
            .trial
            .module_mut()
            .functions
            .iter_mut()
            .find(|function| function.id == prepared.plan.pre_state.function)
            .expect("function")
            .vector_regions
            .insert(0, retained);
        prepared.plan.pre_state.kir_digest = state.kir_digest();
        let original_units = calckernel::kir_function_units(&state.module().functions[0]);
        let transformed_units =
            calckernel::kir_function_units(&prepared.trial.module().functions[0]);
        prepared.plan.pre_state.frozen_kir_units = original_units;
        prepared.plan.growth.original_units = original_units;
        prepared.plan.growth.transformed_units = transformed_units;
        prepared.plan.growth.module_before_units = original_units;
        prepared.plan.growth.module_after_units = transformed_units;
        assert!(
            calckernel::validate_kir_module(state.module())
                .errors
                .is_empty()
        );
        let unchanged = check_vectorization_trial_independently(
            &state,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge,
        );
        assert!(
            unchanged.is_ok(),
            "unchanged prior vector region must survive: {unchanged:?}"
        );
        let function = prepared
            .trial
            .module_mut()
            .functions
            .iter_mut()
            .find(|function| function.id == prepared.plan.pre_state.function)
            .expect("function");
        if remove {
            function.vector_regions.remove(0);
        } else {
            function.vector_regions[0].blocks = vec![function.blocks[0].id];
        }
        let error = check_vectorization_trial_independently(
            &state,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge,
        )
        .expect_err("prior vector region metadata was not frozen");
        assert!(
            format!("{error:?}").contains("pre-existing vector region"),
            "{error:?}"
        );
    }
}

#[test]
fn checker_accepts_distinct_read_places_sharing_a_memory_partition() {
    let (state, prepared) = affine_trial();
    let function = state
        .module()
        .functions
        .iter()
        .find(|function| function.id == prepared.plan.pre_state.function)
        .expect("function");
    let mut reads = Vec::new();
    for instruction in function.blocks.iter().flat_map(|block| &block.instructions) {
        if let calckernel::KirInstructionKind::Load { place } = &instruction.kind
            && let calckernel::KirPlace::SliceIndex { region, .. } = place.as_ref()
        {
            let memory = instruction.memory.as_ref().expect("MemorySSA");
            let descriptor = function
                .regions
                .iter()
                .find(|descriptor| descriptor.id == *region)
                .expect("place region");
            assert_eq!(descriptor.partition, memory.region);
            reads.push((*region, memory.region));
        }
    }
    assert!(
        reads.iter().enumerate().any(|(index, left)| reads
            .iter()
            .skip(index + 1)
            .any(|right| left.0 != right.0 && left.1 == right.1)),
        "the fixture must exercise distinct readonly slices in one alias partition"
    );
}

#[test]
fn checker_rejects_using_place_region_as_broadcast_memory_partition() {
    let (state, mut prepared) = affine_trial();
    let load = prepared.plan.broadcast_groups[0].emitted_scalar_load;
    let instruction = prepared
        .trial
        .module_mut()
        .functions
        .iter_mut()
        .flat_map(|function| &mut function.blocks)
        .flat_map(|block| &mut block.instructions)
        .find(|instruction| instruction.id == load)
        .expect("broadcast load");
    let place_region = match &instruction.kind {
        calckernel::KirInstructionKind::Load { place } => match place.as_ref() {
            calckernel::KirPlace::SliceIndex { region, .. } => *region,
            _ => panic!("slice index"),
        },
        _ => panic!("scalar load"),
    };
    let memory = instruction.memory.as_mut().expect("MemorySSA");
    assert_ne!(place_region, memory.region);
    memory.region = place_region;
    let error = check_vectorization_trial_independently(
        &state,
        &prepared.trial,
        &prepared.plan,
        &prepared.charge,
    )
    .expect_err("forged MemorySSA partition must be rejected");
    assert!(
        format!("{error:?}").contains("memory chain differs"),
        "{error:?}"
    );
}

#[test]
fn checker_rejects_a_different_slice_in_the_same_memory_partition() {
    let (state, mut prepared) = affine_trial();
    let function = state
        .module()
        .functions
        .iter()
        .find(|function| function.id == prepared.plan.pre_state.function)
        .expect("source");
    let source_instruction = |id| {
        function
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .find(|instruction| instruction.id == id)
            .expect("instruction")
    };
    let broadcast = &prepared.plan.broadcast_groups[0];
    let partition = source_instruction(broadcast.scalar_instruction)
        .memory
        .as_ref()
        .expect("broadcast memory")
        .region;
    let vector_id = prepared
        .plan
        .memory_groups
        .iter()
        .find(|group| {
            group.access == calckernel::VectorMemoryAccessKind::Read
                && source_instruction(group.scalar_instructions[0])
                    .memory
                    .as_ref()
                    .is_some_and(|memory| memory.region == partition)
        })
        .expect("other readonly slice in same partition")
        .vector_instruction;
    let wrong_slice = function
        .params
        .iter()
        .find(|param| param.name == "a")
        .expect("a")
        .value;
    let instruction = prepared
        .trial
        .module_mut()
        .functions
        .iter_mut()
        .flat_map(|function| &mut function.blocks)
        .flat_map(|block| &mut block.instructions)
        .find(|instruction| instruction.id == vector_id)
        .expect("vector load");
    if let calckernel::KirInstructionKind::VectorLoad { access, .. } = &mut instruction.kind {
        assert_ne!(access.slice, wrong_slice);
        access.slice = wrong_slice;
    }
    let validation = calckernel::validate_kir_module(prepared.trial.module());
    assert!(
        validation.errors.is_empty(),
        "mutation must remain well-typed KIR: {:?}",
        validation.errors
    );
    let error = check_vectorization_trial_independently(
        &state,
        &prepared.trial,
        &prepared.plan,
        &prepared.charge,
    )
    .expect_err("same-partition slice substitution must not be accepted");
    assert!(
        format!("{error:?}").contains("descriptor-origin root"),
        "{error:?}"
    );
}

fn reject_loop_varying_bound_replacement(replacement: &str, through_body_edge: bool) {
    let (mut state, mut prepared) = affine_trial();
    let function_id = prepared.plan.pre_state.function;
    let induction = match prepared.plan.epilogue {
        calckernel::VectorEpilogue::Scalar { start, .. } => start,
        _ => panic!("scalar tail"),
    };
    let original = state
        .module()
        .functions
        .iter()
        .find(|function| function.id == function_id)
        .expect("source");
    let header = original
        .blocks
        .iter()
        .find(|block| block.params.iter().any(|param| param.value == induction))
        .expect("scalar header");
    let (condition, body_edge) = match &header.terminator {
        calckernel::KirTerminator::Branch {
            condition,
            then_edge,
            ..
        } => (*condition, then_edge),
        _ => panic!("branch"),
    };
    let bound = header
        .instructions
        .iter()
        .find_map(|instruction| match instruction.kind {
            calckernel::KirInstructionKind::Compare { right, .. }
                if instruction
                    .results
                    .iter()
                    .any(|result| result.value == condition) =>
            {
                Some(right)
            }
            _ => None,
        })
        .expect("comparison bound");
    let bound_index = header
        .params
        .iter()
        .position(|param| param.value == bound)
        .expect("header bound parameter");
    let header_replacement_index = header
        .params
        .iter()
        .position(|param| param.slot == replacement)
        .expect("replacement header parameter");
    let body = original
        .blocks
        .iter()
        .find(|block| block.id == body_edge.target)
        .expect("scalar body");
    let body_bound_index = body_edge
        .args
        .iter()
        .position(|argument| *argument == bound)
        .expect("body bound argument");
    let body_replacement_index = body
        .params
        .iter()
        .position(|param| param.slot == replacement)
        .expect("replacement body parameter");
    assert_eq!(
        header.params[bound_index].type_node,
        header.params[header_replacement_index].type_node
    );
    let header_id = header.id;
    let body_id = body.id;
    for module in [state.module_mut(), prepared.trial.module_mut()] {
        let function = module
            .functions
            .iter_mut()
            .find(|function| function.id == function_id)
            .expect("function");
        if through_body_edge {
            let header = function
                .blocks
                .iter_mut()
                .find(|block| block.id == header_id)
                .expect("header");
            let replacement = header.params[header_replacement_index].value;
            if let calckernel::KirTerminator::Branch { then_edge, .. } = &mut header.terminator {
                assert_ne!(then_edge.args[body_bound_index], replacement);
                then_edge.args[body_bound_index] = replacement;
            }
        } else {
            let body = function
                .blocks
                .iter_mut()
                .find(|block| block.id == body_id)
                .expect("body");
            let replacement = body.params[body_replacement_index].value;
            if let calckernel::KirTerminator::Jump { edge } = &mut body.terminator {
                assert_ne!(edge.args[bound_index], replacement);
                edge.args[bound_index] = replacement;
            }
        }
    }
    let function = prepared
        .trial
        .module_mut()
        .functions
        .iter_mut()
        .find(|function| function.id == function_id)
        .expect("trial function");
    if through_body_edge {
        let header = function
            .blocks
            .iter_mut()
            .find(|block| block.label == "loop_simd_header")
            .expect("vector header");
        let replacement = header.params[header_replacement_index].value;
        if let calckernel::KirTerminator::Branch { then_edge, .. } = &mut header.terminator {
            then_edge.args[body_bound_index] = replacement;
        }
    } else {
        let body = function
            .blocks
            .iter_mut()
            .find(|block| block.label == "loop_simd_body")
            .expect("vector body");
        let replacement = body.params[body_replacement_index].value;
        if let calckernel::KirTerminator::Jump { edge } = &mut body.terminator {
            edge.args[bound_index] = replacement;
        }
    }
    prepared.plan.pre_state.kir_digest = state.kir_digest();
    for module in [state.module(), prepared.trial.module()] {
        let validation = calckernel::validate_kir_module(module);
        assert!(
            validation.errors.is_empty(),
            "adversarial fixture must remain valid KIR: {:?}",
            validation.errors
        );
    }
    let result = check_vectorization_trial_independently(
        &state,
        &prepared.trial,
        &prepared.plan,
        &prepared.charge,
    );
    assert!(
        result.is_err(),
        "a loop bound replaced by {replacement} after entry was accepted (body edge={through_body_edge})"
    );
}

#[test]
fn checker_rejects_backedge_bound_replaced_by_broadcast_index() {
    reject_loop_varying_bound_replacement("column", false);
}

#[test]
fn checker_rejects_backedge_bound_replaced_by_another_u32_parameter() {
    reject_loop_varying_bound_replacement("offset", false);
}

#[test]
fn checker_rejects_bound_changed_on_header_to_body_forwarding_edge() {
    reject_loop_varying_bound_replacement("column", true);
}

const NESTED_MATMUL: &str = r#"
export unsafe fn matmul(a: slice<f64>, b: slice<f64>, out: slice<f64>, n: u32) -> void
contract {
  requires n != 0;
  requires n <= a.len && n <= b.len && n <= out.len;
  requires a.len == b.len && b.len == out.len;
  requires noalias(a, b) && noalias(a, out) && noalias(b, out);
  effects read(a), read(b), readwrite(out);
} {
  let i: u32 = 0;
  while i < out.len { out[i] = 0.0; i = i + 1; }
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

fn nested_affine_trial() -> (KirVerifiedProgramState, calckernel::PreparedVectorization) {
    // Build the scalar O3 shape without contract facts, then attach the same
    // source facts for direct trial testing. The production O3 pipeline now
    // accepts this candidate, so it cannot itself serve as the scalar pre-state
    // for adversarial checker mutations.
    let checked = check(&SourceFile::new("nested-affine-checker.ck", NESTED_MATMUL));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("nested MIR");
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
    .expect("nested KIR");
    let optimized = run_kir_pass_pipeline(module, KirOptimizationLevel::O3, None);
    assert!(optimized.errors.is_empty(), "{:?}", optimized.errors);
    let scalar = optimized.artifact.expect("scalar nested O3");
    let contracts = import_contract_facts(&scalar, &checked.checked_program, 0)
        .expect("nested source contracts");
    let state = KirVerifiedProgramState::from_parts(
        scalar,
        Some(contracts),
        optimized.proofs,
        optimized.eliminated_guards,
        0,
    )
    .expect("nested verified scalar pre-state");
    let discovery = discover_vectorization_candidates(&state);
    let candidate = discovery
        .candidates
        .iter()
        .find(|candidate| candidate.wasm_affine.is_some())
        .expect("nested affine candidate");
    let prepared = prepare_vectorization_trial(&state, candidate).expect("nested affine trial");
    assert!(
        calckernel::validate_kir_module(prepared.trial.module())
            .errors
            .is_empty()
    );
    (state, prepared)
}

#[test]
fn checker_accepts_nested_matmul_with_all_path_noalias_forwarding() {
    let (state, prepared) = nested_affine_trial();
    let result = check_vectorization_trial_independently(
        &state,
        &prepared.trial,
        &prepared.plan,
        &prepared.charge,
    );
    assert!(
        result.is_ok(),
        "legitimate nested matmul must retain its proven entry aliases: {result:?}"
    );
}

#[test]
fn checker_rejects_one_changed_outer_loop_slice_incoming_path() {
    let (mut state, mut prepared) = nested_affine_trial();
    let baseline = check_vectorization_trial_independently(
        &state,
        &prepared.trial,
        &prepared.plan,
        &prepared.charge,
    );
    assert!(
        baseline.is_ok(),
        "unmodified nested trial must pass: {baseline:?}"
    );
    let function_id = prepared.plan.pre_state.function;
    let function = state
        .module()
        .functions
        .iter()
        .find(|function| function.id == function_id)
        .expect("function");
    let induction = match prepared.plan.epilogue {
        calckernel::VectorEpilogue::Scalar { start, .. } => start,
        _ => panic!("tail"),
    };
    let inner_header = function
        .blocks
        .iter()
        .find(|block| block.params.iter().any(|param| param.value == induction))
        .expect("inner header")
        .id;
    let dominators = calckernel::compute_kir_dominators(function);
    let mutation = function
        .blocks
        .iter()
        .find_map(|block| {
            let calckernel::KirTerminator::Jump { edge } = &block.terminator else {
                return None;
            };
            if edge.target == inner_header || !dominators.dominates(edge.target, block.id) {
                return None;
            }
            let target = function
                .blocks
                .iter()
                .find(|target| target.id == edge.target)?;
            let a = target.params.iter().position(|param| param.slot == "a")?;
            let out = target.params.iter().position(|param| param.slot == "out")?;
            (edge.args[a] != edge.args[out]).then_some((block.id, a, out))
        })
        .expect("outer backedge carrying both slice descriptors");
    for module in [state.module_mut(), prepared.trial.module_mut()] {
        let block = module
            .functions
            .iter_mut()
            .find(|function| function.id == function_id)
            .expect("function")
            .blocks
            .iter_mut()
            .find(|block| block.id == mutation.0)
            .expect("outer latch");
        let calckernel::KirTerminator::Jump { edge } = &mut block.terminator else {
            panic!("outer backedge")
        };
        edge.args[mutation.1] = edge.args[mutation.2];
    }
    prepared.plan.pre_state.kir_digest = state.kir_digest();
    for module in [state.module(), prepared.trial.module()] {
        let validation = calckernel::validate_kir_module(module);
        assert!(
            validation.errors.is_empty(),
            "outer-path mutation must remain valid KIR: {:?}",
            validation.errors
        );
    }
    let error = check_vectorization_trial_independently(
        &state,
        &prepared.trial,
        &prepared.plan,
        &prepared.charge,
    )
    .expect_err("one changed incoming slice must invalidate the NoAlias forwarding proof");
    assert!(format!("{error:?}").contains("alias freedom"), "{error:?}");
}
