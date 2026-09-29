use calckernel::{
    KirBoundsMode, KirBuildConfig, KirInstructionKind, KirOptimizationLevel, KirOverflowMode,
    KirSanitizerMode, KirTargetProfile, KirVerifiedProgramState, KirWasmFeatures, SourceFile,
    build_kir_module_with_profile, check, check_decision_tree_vector_trial_independently,
    discover_wasm_decision_tree_candidates, import_contract_facts, lower_to_mir,
    prepare_decision_tree_vector_trial, run_kir_pass_pipeline, validate_kir_module,
};

const TREE: &str = r#"
export unsafe fn tree(input: slice<f64>, out: slice<f64>, n: u32) -> void
contract { requires noalias(input, out); effects read(input), write(out); }
{
  let i: u32 = 0;
  while i < n {
    let x: f64 = input[i];
    if x < 0.0 { out[i] = x * 0.25; i = i + 1; continue; }
    if x < 1.0 { out[i] = x * x; i = i + 1; continue; }
    out[i] = x * 0.5 + 0.5;
    i = i + 1;
  }
}
"#;

fn source_state(source: &str) -> KirVerifiedProgramState {
    let checked = check(&SourceFile::new("decision-tree.ck", source));
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
    // Optimize the genuine scalar O3 shape before supplying its NoAlias
    // contract. This prevents the production frontier from consuming the
    // immutable source that the independent mutation tests need.
    let mut optimized = run_kir_pass_pipeline(module, KirOptimizationLevel::O3, None);
    assert!(optimized.errors.is_empty(), "{:?}", optimized.errors);
    let module = optimized.artifact.take().expect("optimized scalar KIR");
    let contracts =
        import_contract_facts(&module, &checked.checked_program, 0).expect("late source contracts");
    KirVerifiedProgramState::from_parts(
        module,
        Some(contracts),
        optimized.proofs,
        optimized.eliminated_guards,
        0,
    )
    .expect("verified state")
}

fn checked_trial() -> (
    KirVerifiedProgramState,
    calckernel::MaterializedDecisionTreeVector,
) {
    checked_trial_for(TREE)
}

fn checked_trial_for(
    source: &str,
) -> (
    KirVerifiedProgramState,
    calckernel::MaterializedDecisionTreeVector,
) {
    checked_trial_for_uf(source, 1)
}

fn checked_trial_for_uf(
    source: &str,
    uf: u8,
) -> (
    KirVerifiedProgramState,
    calckernel::MaterializedDecisionTreeVector,
) {
    let state = source_state(source);
    let candidates = discover_wasm_decision_tree_candidates(&state);
    let candidate = candidates
        .candidates
        .iter()
        .find(|candidate| candidate.uf == uf)
        .unwrap_or_else(|| panic!("closed tree candidate: {candidates:?}"));
    let prepared = prepare_decision_tree_vector_trial(&state, candidate).expect("tree trial");
    let result = check_decision_tree_vector_trial_independently(
        &state,
        &prepared.trial,
        &prepared.plan,
        &prepared.charge,
    );
    assert!(
        result.is_ok(),
        "genuine strict tree trial must pass: {result:?}"
    );
    (state, prepared)
}

#[test]
fn independent_checker_does_not_call_tree_proposal_or_materialization_helpers() {
    let checker = include_str!("../../src/optimizer/decision_tree_vector_check.rs");
    for forbidden in [
        "discover_wasm_decision_tree_candidates",
        "prepare_decision_tree_vector_trial",
        "piecewise_closed_store_tree",
        "analyze_loop_dependences",
    ] {
        assert!(
            !checker.contains(forbidden),
            "independent checker must not call {forbidden}"
        );
    }
}

#[test]
fn accepts_exact_strict_tree_and_two_range_guards() {
    let (_, prepared) = checked_trial();
    assert_eq!(prepared.plan.selects.len(), 2);
    assert_eq!(prepared.plan.vector.predicates.len(), 3);
    assert_eq!(prepared.plan.vector.memory_groups.len(), 2);
}

#[test]
fn materializes_checked_uf4_before_retaining_uf1_fallback() {
    let state = source_state(TREE);
    let discovery = discover_wasm_decision_tree_candidates(&state);
    let uf4 = discovery
        .candidates
        .iter()
        .find(|candidate| candidate.uf == 4)
        .expect("discovery should offer the measured four-chunk unroll");
    assert_eq!(uf4.vf, 2);
    assert!(uf4.minimum_trip >= 8 && uf4.minimum_trip.is_multiple_of(8));
    assert!(
        discovery
            .candidates
            .iter()
            .any(|candidate| candidate.loop_id == uf4.loop_id && candidate.uf == 1),
        "the known VF2/UF1 path remains available as a fallback"
    );

    let prepared = prepare_decision_tree_vector_trial(&state, uf4).expect("UF4 trial");
    assert_eq!(prepared.plan.vector.vf, 2);
    assert_eq!(prepared.plan.vector.uf, 4);
    assert_eq!(prepared.plan.vector.memory_groups.len(), 8);
    for access in [
        calckernel::VectorMemoryAccessKind::Read,
        calckernel::VectorMemoryAccessKind::Write,
    ] {
        let mut unroll_indices = prepared
            .plan
            .vector
            .memory_groups
            .iter()
            .filter(|group| group.access == access)
            .map(|group| group.unroll_index)
            .collect::<Vec<_>>();
        unroll_indices.sort_unstable();
        assert_eq!(unroll_indices, [0, 1, 2, 3]);
    }
    let scalar_ops = prepared
        .plan
        .vector
        .operations
        .iter()
        .map(|mapping| mapping.scalar)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        prepared.plan.vector.operations.len(),
        scalar_ops.len() * 4,
        "every source operation is emitted once per unrolled chunk"
    );
    assert_eq!(
        prepared.plan.selects.len(),
        2 * 4,
        "every source decision is materialized once per unrolled chunk"
    );
    assert!(
        validate_kir_module(prepared.trial.module())
            .errors
            .is_empty()
    );
    check_decision_tree_vector_trial_independently(
        &state,
        &prepared.trial,
        &prepared.plan,
        &prepared.charge,
    )
    .expect("independent UF4 checker");
}

#[test]
fn uf4_checker_rejects_a_corrupted_chunk_offset() {
    let (state, mut prepared) = checked_trial_for_uf(TREE, 4);
    let group = prepared
        .plan
        .vector
        .memory_groups
        .iter()
        .find(|group| {
            group.access == calckernel::VectorMemoryAccessKind::Read && group.unroll_index == 1
        })
        .expect("second chunk load");
    let load = prepared
        .trial
        .module()
        .functions
        .iter()
        .flat_map(|function| &function.blocks)
        .flat_map(|block| &block.instructions)
        .find(|instruction| instruction.id == group.vector_instruction)
        .expect("second load instruction");
    let KirInstructionKind::VectorLoad { access, .. } = &load.kind else {
        panic!("vector load")
    };
    let start = access.start;
    let offset = prepared
        .trial
        .module()
        .functions
        .iter()
        .flat_map(|function| &function.blocks)
        .flat_map(|block| &block.instructions)
        .find(|instruction| {
            instruction
                .results
                .iter()
                .any(|result| result.value == start)
        })
        .expect("chunk offset");
    let KirInstructionKind::Binary { right, .. } = offset.kind else {
        panic!("offset add")
    };
    let constant = prepared
        .trial
        .module_mut()
        .functions
        .iter_mut()
        .flat_map(|function| &mut function.blocks)
        .flat_map(|block| &mut block.instructions)
        .find(|instruction| {
            instruction
                .results
                .iter()
                .any(|result| result.value == right)
        })
        .expect("offset constant");
    let KirInstructionKind::ConstInt { value } = &mut constant.kind else {
        panic!("u32 constant")
    };
    *value = "3".to_string();
    assert!(
        validate_kir_module(prepared.trial.module())
            .errors
            .is_empty()
    );
    assert!(
        check_decision_tree_vector_trial_independently(
            &state,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge
        )
        .is_err()
    );
}

#[test]
fn uf4_checker_rejects_a_broken_output_memory_chain() {
    let (state, mut prepared) = checked_trial_for_uf(TREE, 4);
    let first = prepared
        .plan
        .vector
        .memory_groups
        .iter()
        .find(|group| {
            group.access == calckernel::VectorMemoryAccessKind::Write && group.unroll_index == 0
        })
        .expect("first chunk store");
    let second = prepared
        .plan
        .vector
        .memory_groups
        .iter()
        .find(|group| {
            group.access == calckernel::VectorMemoryAccessKind::Write && group.unroll_index == 1
        })
        .expect("second chunk store");
    let first_instruction = prepared
        .trial
        .module()
        .functions
        .iter()
        .flat_map(|function| &function.blocks)
        .flat_map(|block| &block.instructions)
        .find(|instruction| instruction.id == first.vector_instruction)
        .expect("first store");
    let first_input = first_instruction
        .memory
        .as_ref()
        .expect("first store memory")
        .input;
    let second_instruction = prepared
        .trial
        .module_mut()
        .functions
        .iter_mut()
        .flat_map(|function| &mut function.blocks)
        .flat_map(|block| &mut block.instructions)
        .find(|instruction| instruction.id == second.vector_instruction)
        .expect("second store");
    second_instruction
        .memory
        .as_mut()
        .expect("second store memory")
        .input = first_input;
    assert!(
        validate_kir_module(prepared.trial.module())
            .errors
            .is_empty()
    );
    assert!(
        check_decision_tree_vector_trial_independently(
            &state,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge
        )
        .is_err()
    );
}

#[test]
fn uf4_checker_rejects_a_shortened_trip_guard() {
    let (state, mut prepared) = checked_trial_for_uf(TREE, 4);
    let threshold = prepared
        .plan
        .vector
        .predicates
        .iter_mut()
        .find_map(|predicate| match predicate {
            calckernel::VectorPredicate::TripThreshold { minimum, .. } => Some(minimum),
            _ => None,
        })
        .expect("trip threshold");
    *threshold = 2;
    let function_id = prepared.plan.vector.pre_state.function;
    let function = prepared
        .trial
        .module_mut()
        .functions
        .iter_mut()
        .find(|function| function.id == function_id)
        .expect("transformed function");
    let guard = function
        .blocks
        .iter_mut()
        .flat_map(|block| &mut block.instructions)
        .find(|instruction| {
            matches!(
                instruction.kind,
                KirInstructionKind::VersionPredicate { .. }
            )
        })
        .expect("entry guard");
    let KirInstructionKind::VersionPredicate { predicate } = &mut guard.kind else {
        panic!("version predicate")
    };
    let trip = predicate
        .conjuncts
        .iter_mut()
        .find_map(|conjunct| match conjunct {
            calckernel::KirVersionPredicateConjunct::TripThreshold { minimum, .. } => Some(minimum),
            _ => None,
        })
        .expect("guard threshold");
    *trip = 2;
    assert!(
        validate_kir_module(prepared.trial.module())
            .errors
            .is_empty()
    );
    assert!(
        check_decision_tree_vector_trial_independently(
            &state,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge
        )
        .is_err()
    );
}

#[test]
fn uf4_checker_rejects_a_reduced_checker_charge() {
    let (state, mut prepared) = checked_trial_for_uf(TREE, 4);
    prepared.charge.checker_units = prepared.charge.checker_units.saturating_sub(1);
    assert!(
        check_decision_tree_vector_trial_independently(
            &state,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge
        )
        .is_err()
    );
}

#[test]
fn accepts_four_leaf_tree_with_stable_slice_length_bound() {
    let source = TREE.replace("while i < n", "while i < input.len").replace(
        "out[i] = x * 0.5 + 0.5;",
        "if x < 2.0 { out[i] = x * x * x; i = i + 1; continue; } out[i] = x * 0.5 + 0.5;",
    );
    let (_, prepared) = checked_trial_for(&source);
    assert_eq!(prepared.plan.selects.len(), 3);
    let stores = prepared
        .plan
        .vector
        .memory_groups
        .iter()
        .find(|group| group.access == calckernel::VectorMemoryAccessKind::Write)
        .unwrap();
    assert_eq!(stores.scalar_instructions.len(), 4);
}

#[test]
fn rejects_dropped_negation_on_a_signed_zero_path() {
    let source = TREE.replace("out[i] = x * x;", "out[i] = -x * 0.0;");
    let (state, mut prepared) = checked_trial_for(&source);
    let negate = prepared
        .trial
        .module_mut()
        .functions
        .iter_mut()
        .flat_map(|function| &mut function.blocks)
        .flat_map(|block| &mut block.instructions)
        .find(|instruction| matches!(instruction.kind, KirInstructionKind::VectorUnary { .. }))
        .unwrap()
        .clone();
    let KirInstructionKind::VectorUnary { op, operand, .. } = negate.kind else {
        unreachable!()
    };
    assert_eq!(op, calckernel::KirVectorUnaryOp::Negate);
    let result = negate.results[0].value;
    let multiply = prepared.trial.module_mut().functions.iter_mut()
        .flat_map(|function| &mut function.blocks).flat_map(|block| &mut block.instructions)
        .find(|instruction| matches!(instruction.kind, KirInstructionKind::VectorBinary { left, .. } if left == result)).unwrap();
    let KirInstructionKind::VectorBinary { left, .. } = &mut multiply.kind else {
        unreachable!()
    };
    *left = operand;
    assert!(
        validate_kir_module(prepared.trial.module())
            .errors
            .is_empty()
    );
    assert!(
        check_decision_tree_vector_trial_independently(
            &state,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge
        )
        .is_err()
    );
}

#[test]
fn rejects_swapped_select_arms_in_otherwise_valid_kir() {
    let (state, mut prepared) = checked_trial();
    let id = prepared.plan.selects[0].vector_select;
    let instruction = prepared
        .trial
        .module_mut()
        .functions
        .iter_mut()
        .flat_map(|function| &mut function.blocks)
        .flat_map(|block| &mut block.instructions)
        .find(|instruction| instruction.id == id)
        .expect("select");
    let KirInstructionKind::VectorSelect {
        when_true,
        when_false,
        ..
    } = &mut instruction.kind
    else {
        panic!("select mapping")
    };
    assert_ne!(when_true, when_false);
    std::mem::swap(when_true, when_false);
    assert!(
        validate_kir_module(prepared.trial.module())
            .errors
            .is_empty()
    );
    assert!(
        check_decision_tree_vector_trial_independently(
            &state,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge
        )
        .is_err()
    );
}

#[test]
fn rejects_relational_compare_substitution_that_changes_nan_semantics() {
    let (state, mut prepared) = checked_trial();
    let instruction = prepared
        .trial
        .module_mut()
        .functions
        .iter_mut()
        .flat_map(|function| &mut function.blocks)
        .flat_map(|block| &mut block.instructions)
        .find(|instruction| matches!(instruction.kind, KirInstructionKind::VectorCompare { .. }))
        .expect("compare");
    let KirInstructionKind::VectorCompare { op, .. } = &mut instruction.kind else {
        unreachable!()
    };
    assert_eq!(*op, calckernel::MirCompareOp::Lt);
    *op = calckernel::MirCompareOp::Ge;
    assert!(
        validate_kir_module(prepared.trial.module())
            .errors
            .is_empty()
    );
    assert!(
        check_decision_tree_vector_trial_independently(
            &state,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge
        )
        .is_err()
    );
}

#[test]
fn rejects_changed_vector_store_address() {
    let (state, mut prepared) = checked_trial();
    let instruction = prepared
        .trial
        .module_mut()
        .functions
        .iter_mut()
        .flat_map(|function| &mut function.blocks)
        .flat_map(|block| &mut block.instructions)
        .find(|instruction| matches!(instruction.kind, KirInstructionKind::VectorStore { .. }))
        .expect("store");
    let KirInstructionKind::VectorStore { access, .. } = &mut instruction.kind else {
        unreachable!()
    };
    assert_ne!(access.start, access.end);
    access.start = access.end;
    assert!(
        validate_kir_module(prepared.trial.module())
            .errors
            .is_empty()
    );
    assert!(
        check_decision_tree_vector_trial_independently(
            &state,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge
        )
        .is_err()
    );
}

#[test]
fn rejects_missing_or_shortened_memory_extent_guard() {
    let (state, mut prepared) = checked_trial();
    let instruction = prepared
        .trial
        .module_mut()
        .functions
        .iter_mut()
        .flat_map(|function| &mut function.blocks)
        .flat_map(|block| &mut block.instructions)
        .find(|instruction| {
            matches!(
                instruction.kind,
                KirInstructionKind::VersionPredicate { .. }
            )
        })
        .expect("guard");
    let KirInstructionKind::VersionPredicate { predicate } = &mut instruction.kind else {
        unreachable!()
    };
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
    if let calckernel::KirVersionPredicateConjunct::WasmSliceRange { start, count, .. } = range {
        *count = *start;
    }
    assert!(
        validate_kir_module(prepared.trial.module())
            .errors
            .is_empty()
    );
    assert!(
        check_decision_tree_vector_trial_independently(
            &state,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge
        )
        .is_err()
    );
}

#[test]
fn rejects_non_dominating_noalias_evidence() {
    let (mut state, mut prepared) = checked_trial();
    let function = state
        .module()
        .functions
        .iter()
        .find(|function| function.id == prepared.plan.vector.pre_state.function)
        .expect("function");
    let exit = function
        .blocks
        .iter()
        .find(|block| matches!(block.terminator, calckernel::KirTerminator::Return { .. }))
        .expect("exit")
        .id;
    let function_id = function.id;
    let facts = state.contract_facts_mut().expect("facts").facts_mut();
    let fact = facts
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
    facts.get_mut(fact).expect("fact").scope = calckernel::FactScope::Block {
        function: function_id,
        block: exit,
    };
    prepared
        .trial
        .contract_facts_mut()
        .expect("trial facts")
        .facts_mut()
        .get_mut(fact)
        .expect("trial fact")
        .scope = calckernel::FactScope::Block {
        function: function_id,
        block: exit,
    };
    assert!(
        check_decision_tree_vector_trial_independently(
            &state,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge
        )
        .is_err()
    );
}

#[test]
fn rejects_unused_trapping_division_in_source_loop_header() {
    let state = source_state(TREE);
    let candidate = discover_wasm_decision_tree_candidates(&state)
        .candidates
        .remove(0);
    let mut module = state.module().clone();
    let next_instruction = module
        .functions
        .iter()
        .flat_map(|function| &function.blocks)
        .flat_map(|block| &block.instructions)
        .map(|instruction| instruction.id.index())
        .max()
        .unwrap()
        + 1;
    let next_value = module
        .functions
        .iter()
        .flat_map(|function| {
            function
                .params
                .iter()
                .map(|param| param.value.index())
                .chain(function.blocks.iter().flat_map(|block| {
                    block.params.iter().map(|param| param.value.index()).chain(
                        block.instructions.iter().flat_map(|instruction| {
                            instruction
                                .results
                                .iter()
                                .map(|result| result.value.index())
                        }),
                    )
                }))
        })
        .max()
        .unwrap()
        + 1;
    let header = module
        .functions
        .iter_mut()
        .find(|function| function.id == candidate.function)
        .unwrap()
        .blocks
        .iter_mut()
        .find(|block| block.id == candidate.header)
        .unwrap();
    header.instructions.push(calckernel::KirInstruction {
        id: calckernel::InstructionId::from_index(next_instruction),
        results: vec![calckernel::KirResult {
            value: calckernel::ValueId::from_index(next_value),
            type_node: calckernel::KirValueType::Scalar(calckernel::MirType::Primitive(
                calckernel::MirPrimitiveTypeName::U32,
            )),
        }],
        kind: KirInstructionKind::Binary {
            op: calckernel::MirBinaryOp::Div,
            left: candidate.induction,
            right: candidate.induction,
            semantics: calckernel::KirArithmeticSemantics::Modular,
        },
        memory: None,
        effect: None,
    });
    assert!(validate_kir_module(&module).errors.is_empty());
    // Rebuild the verified state so the materializer's allocator starts above
    // the injected IDs. At the first iteration this unused division traps.
    let mutated = KirVerifiedProgramState::from_parts(
        module,
        state.contract_facts().cloned(),
        state.proofs().clone(),
        state.eliminated_guards().to_vec(),
        state.evidence_generation(),
    )
    .expect("valid mutated KIR state");
    let prepared = prepare_decision_tree_vector_trial(&mutated, &candidate)
        .expect("the proposer candidate still materializes a trial");
    let result = check_decision_tree_vector_trial_independently(
        &mutated,
        &prepared.trial,
        &prepared.plan,
        &prepared.charge,
    );
    assert!(
        matches!(&result, Err(error) if format!("{error:?}").contains("skipped or trapping work")),
        "checker must reject the source header trap skipped by vector entry: {result:?}"
    );
}

#[test]
fn rejects_loop_bound_changed_on_source_and_vector_backedges() {
    let (mut state, mut prepared) = checked_trial();
    let source = discover_wasm_decision_tree_candidates(&state)
        .candidates
        .remove(0);
    let function = state
        .module()
        .functions
        .iter()
        .find(|f| f.id == source.function)
        .unwrap();
    let header = function
        .blocks
        .iter()
        .find(|block| block.id == source.header)
        .unwrap();
    let bound = header
        .instructions
        .iter()
        .find_map(|instruction| match instruction.kind {
            KirInstructionKind::Compare { right, .. } => Some(right),
            _ => None,
        })
        .unwrap();
    let bound_index = header
        .params
        .iter()
        .position(|param| param.value == bound)
        .unwrap();
    let iv_index = header
        .params
        .iter()
        .position(|param| param.value == source.induction)
        .unwrap();
    assert_ne!(bound_index, iv_index);
    for module in [state.module_mut(), prepared.trial.module_mut()] {
        let function = module
            .functions
            .iter_mut()
            .find(|f| f.id == source.function)
            .unwrap();
        let join = function
            .blocks
            .iter_mut()
            .find(|block| block.id == source.join)
            .unwrap();
        let calckernel::KirTerminator::Jump { edge } = &mut join.terminator else {
            unreachable!()
        };
        edge.args[bound_index] = edge.args[iv_index];
    }
    let function = prepared
        .trial
        .module_mut()
        .functions
        .iter_mut()
        .find(|f| f.id == source.function)
        .unwrap();
    let body = function
        .blocks
        .iter_mut()
        .find(|block| {
            block.instructions.iter().any(|instruction| {
                matches!(instruction.kind, KirInstructionKind::VectorStore { .. })
            })
        })
        .unwrap();
    let calckernel::KirTerminator::Jump { edge } = &mut body.terminator else {
        unreachable!()
    };
    edge.args[bound_index] = edge.args[iv_index];
    prepared.plan.vector.pre_state.kir_digest = state.kir_digest();
    for module in [state.module(), prepared.trial.module()] {
        let result = validate_kir_module(module);
        assert!(
            result.errors.is_empty(),
            "bound mutation must remain valid KIR: {:?}",
            result.errors
        );
    }
    let result = check_decision_tree_vector_trial_independently(
        &state,
        &prepared.trial,
        &prepared.plan,
        &prepared.charge,
    );
    assert!(
        matches!(&result, Err(error) if format!("{error:?}").contains("bound")),
        "loop-varying bound must fail the source invariance proof: {result:?}"
    );
}

#[test]
fn rejects_source_effect_order_that_differs_from_root_read_then_store() {
    let (mut state, mut prepared) = checked_trial();
    let source = discover_wasm_decision_tree_candidates(&state)
        .candidates
        .remove(0);
    let max_order = prepared
        .trial
        .module()
        .functions
        .iter()
        .flat_map(|function| &function.blocks)
        .flat_map(|block| &block.instructions)
        .filter_map(|instruction| instruction.effect.as_ref().map(|effect| effect.order))
        .max()
        .unwrap();
    for module in [state.module_mut(), prepared.trial.module_mut()] {
        let function = module
            .functions
            .iter_mut()
            .find(|f| f.id == source.function)
            .unwrap();
        let load = function
            .blocks
            .iter_mut()
            .flat_map(|block| &mut block.instructions)
            .find(|instruction| instruction.id == source.root_load)
            .unwrap();
        load.effect.as_mut().unwrap().order = max_order + 100;
    }
    prepared.plan.vector.pre_state.kir_digest = state.kir_digest();
    for module in [state.module(), prepared.trial.module()] {
        let result = validate_kir_module(module);
        assert!(
            result.errors.is_empty(),
            "effect mutation must remain valid KIR: {:?}",
            result.errors
        );
    }
    let result = check_decision_tree_vector_trial_independently(
        &state,
        &prepared.trial,
        &prepared.plan,
        &prepared.charge,
    );
    assert!(
        matches!(&result, Err(error) if format!("{error:?}").contains("effect order")),
        "source effect-order mutation must fail its independent proof: {result:?}"
    );
}

#[test]
fn rejects_discounted_speculative_work_or_incomplete_checker_charge() {
    let (state, mut prepared) = checked_trial();
    prepared.charge.checker_units = prepared.charge.checker_units.saturating_sub(1);
    assert!(
        check_decision_tree_vector_trial_independently(
            &state,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge
        )
        .is_err()
    );
}

#[test]
fn rejects_all_arm_scalar_pricing_when_root_address_identity_is_not_closed() {
    let (mut state, mut prepared) = checked_trial();
    let store = prepared
        .plan
        .vector
        .memory_groups
        .iter()
        .find(|group| group.access == calckernel::VectorMemoryAccessKind::Write)
        .unwrap()
        .scalar_instructions[0];
    for module in [state.module_mut(), prepared.trial.module_mut()] {
        let function = module
            .functions
            .iter_mut()
            .find(|function| function.id == prepared.plan.vector.pre_state.function)
            .unwrap();
        let output = function
            .params
            .iter()
            .find(|parameter| parameter.name == "out")
            .unwrap()
            .value;
        let instruction = function
            .blocks
            .iter_mut()
            .flat_map(|block| &mut block.instructions)
            .find(|instruction| instruction.id == store)
            .unwrap();
        let KirInstructionKind::Store { place, .. } = &mut instruction.kind else {
            unreachable!()
        };
        let calckernel::KirPlace::SliceIndex { slice, .. } = place.as_mut() else {
            unreachable!()
        };
        assert_ne!(*slice, output);
        *slice = output;
    }
    prepared.plan.vector.pre_state.kir_digest = state.kir_digest();
    for module in [state.module(), prepared.trial.module()] {
        let validation = validate_kir_module(module);
        assert!(
            validation.errors.is_empty(),
            "equivalent descriptor mutation must remain valid KIR: {:?}",
            validation.errors
        );
    }
    let result = check_decision_tree_vector_trial_independently(
        &state,
        &prepared.trial,
        &prepared.plan,
        &prepared.charge,
    );
    assert!(
        result.is_err(),
        "global descriptor equivalence does not prove the scalar backend selects this tree"
    );
}

#[test]
fn rejects_trial_export_and_parameter_identity_changes() {
    let (state, prepared) = checked_trial();
    for swap_parameters in [false, true] {
        let mut changed = prepared.trial.clone();
        let function = changed
            .module_mut()
            .functions
            .iter_mut()
            .find(|function| function.id == prepared.plan.vector.pre_state.function)
            .unwrap();
        if swap_parameters {
            assert_eq!(function.params[0].type_node, function.params[1].type_node);
            function.params.swap(0, 1);
        } else {
            function.exported = !function.exported;
        }
        assert!(validate_kir_module(changed.module()).errors.is_empty());
        assert!(
            check_decision_tree_vector_trial_independently(
                &state,
                &changed,
                &prepared.plan,
                &prepared.charge
            )
            .is_err()
        );
    }
}

#[test]
fn rejects_stale_memory_input_and_dropped_write_on_vector_backedge() {
    let (state, prepared) = checked_trial();
    for drop_write in [false, true] {
        let mut changed = prepared.trial.clone();
        let function = changed
            .module_mut()
            .functions
            .iter_mut()
            .find(|function| function.id == prepared.plan.vector.pre_state.function)
            .unwrap();
        let body_index = function
            .blocks
            .iter()
            .position(|block| {
                block.instructions.iter().any(|instruction| {
                    matches!(instruction.kind, KirInstructionKind::VectorStore { .. })
                })
            })
            .unwrap();
        let store_index = function.blocks[body_index]
            .instructions
            .iter()
            .position(|instruction| {
                matches!(instruction.kind, KirInstructionKind::VectorStore { .. })
            })
            .unwrap();
        let store_memory = function.blocks[body_index].instructions[store_index]
            .memory
            .clone()
            .unwrap();
        if drop_write {
            let calckernel::KirTerminator::Jump { edge } =
                &mut function.blocks[body_index].terminator
            else {
                unreachable!()
            };
            let argument = edge
                .memory_args
                .iter_mut()
                .find(|version| Some(**version) == store_memory.output)
                .unwrap();
            *argument = store_memory.input;
        } else {
            let initial = function
                .initial_memory
                .iter()
                .find(|memory| memory.region == store_memory.region)
                .unwrap()
                .version;
            assert_ne!(store_memory.input, initial);
            function.blocks[body_index].instructions[store_index]
                .memory
                .as_mut()
                .unwrap()
                .input = initial;
        }
        let validation = validate_kir_module(changed.module());
        assert!(
            validation.errors.is_empty(),
            "stale-version fixture must remain valid KIR: {:?}",
            validation.errors
        );
        assert!(
            check_decision_tree_vector_trial_independently(
                &state,
                &changed,
                &prepared.plan,
                &prepared.charge
            )
            .is_err()
        );
    }
}
