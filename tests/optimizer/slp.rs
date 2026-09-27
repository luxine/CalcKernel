use std::collections::BTreeSet;

use calckernel::{
    CandidateBudgetCharge, CandidateDisposition, CandidateKey, KirAlignmentClass, KirBoundsMode,
    KirBuildConfig, KirConsumer, KirCostKey, KirCostSemantics, KirLaneType, KirLegalCost,
    KirNativeCpuPolicy, KirOptimizationAuditState, KirOptimizationLevel, KirOverflowMode,
    KirProfileOperation, KirSanitizerMode, KirTargetProfile, KirTargetProfileBuilder,
    KirVerifiedProgramState, SourceFile, TransactionOutcome, analyze_canonical_loops,
    build_kir_module, check, check_slp_plan_independently, check_unroll_plan_independently,
    check_unroll_slp_trial_independently, combined_unroll_slp_cost, discover_slp_candidates,
    discover_unroll_candidates, execute_verified_transaction, import_contract_facts, lower_to_mir,
    prepare_slp_trial, prepare_unroll_trial, print_kir_module, run_kir_pass_pipeline,
    slp_profitability_threshold,
};

const FOUR_WAY: &str = r#"
export fn lanes(a0: i32, a1: i32, a2: i32, a3: i32, b0: i32, b1: i32, b2: i32, b3: i32) -> i32 {
  let p0: i32 = a0 * b0;
  let p1: i32 = a1 * b1;
  let p2: i32 = a2 * b2;
  let p3: i32 = a3 * b3;
  return p0 + p1 + p2 + p3;
}
"#;

fn profile() -> KirTargetProfile {
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
    for operation in [
        KirProfileOperation::Splat,
        KirProfileOperation::Insert,
        KirProfileOperation::Multiply,
        KirProfileOperation::Extract,
    ] {
        let semantics = if operation == KirProfileOperation::Multiply {
            KirCostSemantics::Modular
        } else {
            KirCostSemantics::NotApplicable
        };
        builder
            .set_legal(
                KirCostKey {
                    operation,
                    lane: KirLaneType::I32,
                    lanes: 4,
                    semantics,
                    alignment: KirAlignmentClass::NotApplicable,
                },
                KirLegalCost {
                    cost: 1,
                    legalization_parts: 1,
                    legalized_type: "v4i32".to_string(),
                },
            )
            .unwrap();
    }
    builder
        .set_legal(
            KirCostKey {
                operation: KirProfileOperation::Multiply,
                lane: KirLaneType::I32,
                lanes: 1,
                semantics: KirCostSemantics::Modular,
                alignment: KirAlignmentClass::NotApplicable,
            },
            KirLegalCost {
                cost: 20,
                legalization_parts: 1,
                legalized_type: "i32".to_string(),
            },
        )
        .unwrap();
    for operation in [
        KirProfileOperation::Splat,
        KirProfileOperation::Insert,
        KirProfileOperation::Divide,
        KirProfileOperation::Extract,
    ] {
        let semantics = if operation == KirProfileOperation::Divide {
            KirCostSemantics::StrictFloat
        } else {
            KirCostSemantics::NotApplicable
        };
        builder
            .set_legal(
                KirCostKey {
                    operation,
                    lane: KirLaneType::F64,
                    lanes: 4,
                    semantics,
                    alignment: KirAlignmentClass::NotApplicable,
                },
                KirLegalCost {
                    cost: 1,
                    legalization_parts: 1,
                    legalized_type: "<4 x double>".to_string(),
                },
            )
            .unwrap();
    }
    builder
        .set_legal(
            KirCostKey {
                operation: KirProfileOperation::Divide,
                lane: KirLaneType::F64,
                lanes: 1,
                semantics: KirCostSemantics::StrictFloat,
                alignment: KirAlignmentClass::NotApplicable,
            },
            KirLegalCost {
                cost: 20,
                legalization_parts: 1,
                legalized_type: "double".to_string(),
            },
        )
        .unwrap();
    for lanes in [2_u8, 4] {
        for operation in [
            KirProfileOperation::Load,
            KirProfileOperation::Add,
            KirProfileOperation::Store,
        ] {
            builder
                .set_legal(
                    KirCostKey {
                        operation,
                        lane: KirLaneType::U32,
                        lanes,
                        semantics: if operation == KirProfileOperation::Add {
                            KirCostSemantics::Modular
                        } else {
                            KirCostSemantics::NotApplicable
                        },
                        alignment: if matches!(
                            operation,
                            KirProfileOperation::Load | KirProfileOperation::Store
                        ) {
                            KirAlignmentClass::Bytes(4)
                        } else {
                            KirAlignmentClass::NotApplicable
                        },
                    },
                    KirLegalCost {
                        cost: 1,
                        legalization_parts: 1,
                        legalized_type: format!("<{lanes} x i32>"),
                    },
                )
                .unwrap();
        }
    }
    for operation in [
        KirProfileOperation::Load,
        KirProfileOperation::Add,
        KirProfileOperation::Store,
    ] {
        builder
            .set_legal(
                KirCostKey {
                    operation,
                    lane: KirLaneType::U32,
                    lanes: 1,
                    semantics: if operation == KirProfileOperation::Add {
                        KirCostSemantics::Modular
                    } else {
                        KirCostSemantics::NotApplicable
                    },
                    alignment: if matches!(
                        operation,
                        KirProfileOperation::Load | KirProfileOperation::Store
                    ) {
                        KirAlignmentClass::Bytes(4)
                    } else {
                        KirAlignmentClass::NotApplicable
                    },
                },
                KirLegalCost {
                    cost: 2,
                    legalization_parts: 1,
                    legalized_type: "i32".to_string(),
                },
            )
            .unwrap();
    }
    builder.build().unwrap()
}

fn state(source: &str) -> KirVerifiedProgramState {
    let checked = check(&SourceFile::new("slp.ck", source));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).unwrap();
    let mut module = build_kir_module(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::NativeLibrary,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
    )
    .unwrap();
    module.profile = profile();
    KirVerifiedProgramState::new(module, None, 0).unwrap()
}

fn state_with_contract(source: &str) -> KirVerifiedProgramState {
    let checked = check(&SourceFile::new("memory-slp.ck", source));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).unwrap();
    let mut module = build_kir_module(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::NativeLibrary,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
    )
    .unwrap();
    module.profile = profile();
    let contracts = import_contract_facts(&module, &checked.checked_program, 0).unwrap();
    KirVerifiedProgramState::new(module, Some(contracts), 0).unwrap()
}

fn multiply_candidate(state: &KirVerifiedProgramState) -> calckernel::SlpCandidate {
    discover_slp_candidates(&state.module().functions[0], &BTreeSet::new())
        .candidates
        .into_iter()
        .find(|candidate| {
            candidate.operation == KirProfileOperation::Multiply && candidate.lanes == 4
        })
        .expect("four-way multiply pack")
}

#[test]
fn slp_should_discover_source_order_identity_pack_and_materialize_verified_vector_kir() {
    let pre = state(FOUR_WAY);
    let alternatives = discover_slp_candidates(&pre.module().functions[0], &BTreeSet::new());
    assert!(
        alternatives
            .candidates
            .iter()
            .any(|candidate| candidate.lanes == 2)
    );
    assert!(
        alternatives
            .candidates
            .iter()
            .any(|candidate| candidate.lanes == 4)
    );
    let candidate = multiply_candidate(&pre);
    assert_eq!(candidate.lanes, 4);
    assert_eq!(candidate.scalar_instructions.len(), 4);
    let prepared = prepare_slp_trial(&pre, &candidate).expect("SLP proposal");
    assert_eq!(
        check_slp_plan_independently(&pre, &prepared.trial, &prepared.plan, &prepared.charge,),
        Ok(())
    );
    assert_eq!(prepared.plan.proof.identity_lanes, [0, 1, 2, 3]);
    assert_eq!(prepared.plan.vector_instructions.len(), 9);
    assert_eq!(prepared.plan.extracts.len(), 4);
}

#[test]
fn slp_native_residual_frontier_should_commit_the_verified_pack() {
    let pre = state(FOUR_WAY);
    let scalar = pre.module().clone();
    let result = run_kir_pass_pipeline(scalar.clone(), KirOptimizationLevel::O3, None);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(result.stats.staged_native_slp_candidates, 0);
    assert!(result.stats.slp_packs >= 1);
    assert!(
        result
            .artifact
            .as_ref()
            .unwrap()
            .functions
            .iter()
            .any(|function| !function.vector_regions.is_empty())
    );
    assert!(result.audit.attempts().iter().any(|attempt| {
        matches!(attempt.key, CandidateKey::ResidualSlp { .. })
            && attempt.disposition == CandidateDisposition::Accepted
            && attempt.reason == "accepted"
    }));
}

#[test]
fn slp_should_pack_strict_f64_divide_but_keep_integer_division_as_a_stop_point() {
    let strict = state(
        "export fn lanes(a0: f64, a1: f64, a2: f64, a3: f64, b0: f64, b1: f64, b2: f64, b3: f64) -> f64 { let p0: f64 = a0 / b0; let p1: f64 = a1 / b1; let p2: f64 = a2 / b2; let p3: f64 = a3 / b3; return p0 + p1 + p2 + p3; }",
    );
    let candidate = discover_slp_candidates(&strict.module().functions[0], &BTreeSet::new())
        .candidates
        .into_iter()
        .find(|candidate| {
            candidate.operation == KirProfileOperation::Divide && candidate.lanes == 4
        })
        .expect("strict f64 divide pack");
    let prepared = prepare_slp_trial(&strict, &candidate).expect("strict f64 SLP proposal");
    assert_eq!(
        check_slp_plan_independently(&strict, &prepared.trial, &prepared.plan, &prepared.charge),
        Ok(())
    );
    assert!(
        print_kir_module(prepared.trial.module()).contains("vector_divide.strict"),
        "strict f64 SLP must materialize vector division"
    );

    let integer = state(
        "export fn lanes(a0: u32, a1: u32, a2: u32, a3: u32, b0: u32, b1: u32, b2: u32, b3: u32) -> u32 { let p0: u32 = a0 / b0; let p1: u32 = a1 / b1; let p2: u32 = a2 / b2; let p3: u32 = a3 / b3; return p0 + p1 + p2 + p3; }",
    );
    assert!(
        discover_slp_candidates(&integer.module().functions[0], &BTreeSet::new())
            .candidates
            .iter()
            .all(|candidate| candidate.operation != KirProfileOperation::Divide)
    );
}

#[test]
fn slp_should_pack_contiguous_load_compute_store_with_exact_memory_footprint() {
    let pre = state_with_contract(
        r#"
export unsafe fn lanes(a: slice<u32>, b: slice<u32>, out: slice<u32>) -> void
contract { requires a.len >= 4 && b.len >= 4 && out.len >= 4; requires noalias(a, b) && noalias(a, out) && noalias(b, out); effects read(a), read(b), write(out); }
{
  out[0] = a[0] + b[0];
  out[1] = a[1] + b[1];
  out[2] = a[2] + b[2];
  out[3] = a[3] + b[3];
}
"#,
    );
    let candidate = discover_slp_candidates(&pre.module().functions[0], &BTreeSet::new())
        .candidates
        .into_iter()
        .find(|candidate| candidate.lanes == 4 && candidate.memory.is_some())
        .unwrap_or_else(|| {
            panic!(
                "contiguous memory SLP candidate:\n{}",
                print_kir_module(pre.module())
            )
        });
    let prepared = prepare_slp_trial(&pre, &candidate).expect("memory SLP proposal");
    assert_eq!(
        check_slp_plan_independently(&pre, &prepared.trial, &prepared.plan, &prepared.charge),
        Ok(())
    );
    assert_eq!(prepared.plan.proof.exact_memory_footprint, Some(16));
    let text = print_kir_module(prepared.trial.module());
    assert_eq!(text.matches("vector_load").count(), 2, "{text}");
    assert_eq!(text.matches("vector_store").count(), 1, "{text}");
    let mut committed = pre.clone();
    let mut audit = KirOptimizationAuditState::for_module(pre.module());
    let plan = prepared.plan.clone();
    let charge = prepared.charge.clone();
    let final_state = prepared.trial;
    let outcome = execute_verified_transaction(
        &mut committed,
        &mut audit,
        candidate.key,
        prepared.charge,
        move |trial| {
            *trial = final_state;
            Ok(())
        },
        move |before, after| check_slp_plan_independently(before, after, &plan, &charge),
    );
    assert_eq!(outcome, TransactionOutcome::Committed);

    let production = run_kir_pass_pipeline(
        pre.module().clone(),
        KirOptimizationLevel::O3,
        pre.contract_facts(),
    );
    assert!(production.errors.is_empty(), "{:?}", production.errors);
    let text = print_kir_module(production.artifact.as_ref().expect("SLP artifact"));
    assert!(
        text.contains("vector<u32, 4>"),
        "overlapping residual SLP alternatives must commit the maximal profitable pack:\n{text}"
    );
    assert!(production.audit.attempts().iter().any(|attempt| {
        matches!(attempt.key, CandidateKey::ResidualSlp { lanes: 2, .. })
            && attempt.disposition == CandidateDisposition::NonWinner
    }));
    assert!(production.audit.attempts().iter().any(|attempt| {
        matches!(attempt.key, CandidateKey::ResidualSlp { lanes: 4, .. })
            && attempt.disposition == CandidateDisposition::Accepted
    }));
}

#[test]
fn memory_slp_checker_should_reject_missing_alias_evidence_and_forged_footprint_or_mapping() {
    let source = r#"
export unsafe fn lanes(a: slice<u32>, b: slice<u32>, out: slice<u32>) -> void
contract { requires a.len >= 4 && b.len >= 4 && out.len >= 4; requires noalias(a, b) && noalias(a, out) && noalias(b, out); effects read(a), read(b), write(out); }
{
  out[0] = a[0] + b[0];
  out[1] = a[1] + b[1];
  out[2] = a[2] + b[2];
  out[3] = a[3] + b[3];
}
"#;
    let pre = state_with_contract(source);
    let candidate = discover_slp_candidates(&pre.module().functions[0], &BTreeSet::new())
        .candidates
        .into_iter()
        .find(|candidate| candidate.lanes == 4 && candidate.memory.is_some())
        .expect("memory SLP candidate");
    let prepared = prepare_slp_trial(&pre, &candidate).expect("memory SLP proposal");

    let mut footprint = prepared.plan.clone();
    footprint.proof.exact_memory_footprint = Some(12);
    assert!(
        check_slp_plan_independently(&pre, &prepared.trial, &footprint, &prepared.charge).is_err()
    );
    let mut mapping = prepared.plan.clone();
    let forged_store = mapping.memory.as_ref().unwrap().vector_loads[0];
    mapping.memory.as_mut().unwrap().vector_store = forged_store;
    assert!(
        check_slp_plan_independently(&pre, &prepared.trial, &mapping, &prepared.charge).is_err()
    );
    let mut forged_trial = prepared.trial.clone();
    let vector_load = prepared.plan.memory.as_ref().unwrap().vector_loads[0];
    let instruction = forged_trial
        .module_mut()
        .functions
        .iter_mut()
        .flat_map(|function| &mut function.blocks)
        .flat_map(|block| &mut block.instructions)
        .find(|instruction| instruction.id == vector_load)
        .expect("vector load mapping");
    let calckernel::KirInstructionKind::VectorLoad { access, .. } = &mut instruction.kind else {
        panic!("vector load kind")
    };
    access.byte_footprint = 12;
    assert!(
        check_slp_plan_independently(&pre, &forged_trial, &prepared.plan, &prepared.charge)
            .is_err()
    );

    let no_evidence = state(source);
    let candidate = discover_slp_candidates(&no_evidence.module().functions[0], &BTreeSet::new())
        .candidates
        .into_iter()
        .find(|candidate| candidate.lanes == 4 && candidate.memory.is_some())
        .expect("proposal discovery remains conservative-evidence agnostic");
    let prepared = prepare_slp_trial(&no_evidence, &candidate).expect("memory SLP proposal");
    assert!(
        check_slp_plan_independently(
            &no_evidence,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge,
        )
        .is_err(),
        "the independent checker must require pairwise noalias evidence"
    );
}

#[test]
fn slp_should_stop_at_call_guard_memory_certificate_and_lane_dependence_barriers() {
    let dependency = state(FOUR_WAY);
    let candidate = multiply_candidate(&dependency);
    let protected = BTreeSet::from([candidate.scalar_instructions[1]]);
    assert!(
        discover_slp_candidates(&dependency.module().functions[0], &protected)
            .candidates
            .iter()
            .filter(|item| item.operation == KirProfileOperation::Multiply)
            .all(|item| {
                !item
                    .scalar_instructions
                    .contains(&candidate.scalar_instructions[1])
                    && item.lanes == 2
            })
    );

    let call = state(
        "fn stop(n: i32) -> i32 { return n; } export fn lanes(a0: i32, a1: i32, a2: i32, a3: i32) -> i32 { let p0: i32 = a0 * a0; let p1: i32 = a1 * a1; let x: i32 = stop(p0); let p2: i32 = a2 * a2; let p3: i32 = a3 * a3; return x + p1 + p2 + p3; }",
    );
    let function = &call.module().functions[1];
    let block = &function.blocks[0];
    let call_position = block
        .instructions
        .iter()
        .position(|instruction| {
            matches!(
                instruction.kind,
                calckernel::KirInstructionKind::Call { .. }
            )
        })
        .expect("call barrier");
    assert!(
        discover_slp_candidates(function, &BTreeSet::new())
            .candidates
            .iter()
            .filter(|item| item.operation == KirProfileOperation::Multiply)
            .all(|item| {
                let positions = item
                    .scalar_instructions
                    .iter()
                    .map(|id| {
                        block
                            .instructions
                            .iter()
                            .position(|instruction| instruction.id == *id)
                            .unwrap()
                    })
                    .collect::<Vec<_>>();
                positions.iter().all(|position| *position < call_position)
                    || positions.iter().all(|position| *position > call_position)
            })
    );

    let chain = state(
        "export fn chain(a: i32, b: i32, c: i32) -> i32 { let p0: i32 = a * b; let p1: i32 = p0 * c; return p1; }",
    );
    assert!(
        discover_slp_candidates(&chain.module().functions[0], &BTreeSet::new())
            .candidates
            .is_empty()
    );
}

#[test]
fn slp_should_reject_forged_order_cost_growth_lane_proof_and_vector_mapping() {
    let pre = state(FOUR_WAY);
    let candidate = multiply_candidate(&pre);
    let prepared = prepare_slp_trial(&pre, &candidate).unwrap();

    let mut order = prepared.plan.clone();
    order.scalar_instructions.swap(0, 1);
    assert!(check_slp_plan_independently(&pre, &prepared.trial, &order, &prepared.charge).is_err());
    let mut cost = prepared.plan.clone();
    cost.cost.total += 1;
    assert!(check_slp_plan_independently(&pre, &prepared.trial, &cost, &prepared.charge).is_err());
    let mut growth = prepared.plan.clone();
    growth.growth.module_after_units += 1;
    assert!(
        check_slp_plan_independently(&pre, &prepared.trial, &growth, &prepared.charge).is_err()
    );
    let mut lanes = prepared.plan.clone();
    lanes.proof.identity_lanes.swap(0, 1);
    assert!(check_slp_plan_independently(&pre, &prepared.trial, &lanes, &prepared.charge).is_err());
    let mut vector = prepared.plan.clone();
    vector.vector_instructions[0] = vector.extracts[0];
    assert!(
        check_slp_plan_independently(&pre, &prepared.trial, &vector, &prepared.charge).is_err()
    );
    let mut charge = prepared.charge.clone();
    charge.proposer_units += 1;
    assert!(check_slp_plan_independently(&pre, &prepared.trial, &prepared.plan, &charge).is_err());
}

#[test]
fn slp_profitability_should_require_ten_percent_and_two_units() {
    assert!(slp_profitability_threshold(
        calckernel::KirCostEstimate::new(20, 18, 0, 0)
    ));
    assert!(!slp_profitability_threshold(
        calckernel::KirCostEstimate::new(20, 19, 0, 0)
    ));
}

#[test]
fn unroll_slp_transaction_should_commit_or_rollback_both_halves_atomically() {
    let source = r#"
export fn lanes(a0: i32, a1: i32, a2: i32, a3: i32, b0: i32, b1: i32, b2: i32, b3: i32) -> i32 {
  let i: u32 = 0;
  let p0: i32 = 0; let p1: i32 = 0; let p2: i32 = 0; let p3: i32 = 0;
  while i < 2 {
    p0 = a0 * b0; p1 = a1 * b1; p2 = a2 * b2; p3 = a3 * b3;
    i = i + 1;
  }
  return p0 + p1 + p2 + p3;
}
"#;
    let pre = state(source);
    let function = &pre.module().functions[0];
    let loops = analyze_canonical_loops(function);
    let unroll_candidate = discover_unroll_candidates(function, &loops.loops)
        .candidates
        .remove(0);
    let unroll = prepare_unroll_trial(&pre, &unroll_candidate).expect("unroll half");
    assert_eq!(
        check_unroll_plan_independently(&pre, &unroll.trial, &unroll.plan, &unroll.charge),
        Ok(())
    );
    let slp_candidate = multiply_candidate(&unroll.trial);
    let slp = prepare_slp_trial(&unroll.trial, &slp_candidate).expect("SLP half");
    assert_eq!(
        check_slp_plan_independently(&unroll.trial, &slp.trial, &slp.plan, &slp.charge),
        Ok(())
    );
    let combined_charge = CandidateBudgetCharge::single(
        unroll_candidate.function,
        unroll
            .charge
            .proposer_units
            .saturating_add(slp.charge.proposer_units),
        unroll
            .charge
            .checker_units
            .saturating_add(slp.charge.checker_units),
    );
    let combined_cost = combined_unroll_slp_cost(&unroll.plan, &slp.plan);
    assert_eq!(
        check_unroll_slp_trial_independently(
            &pre,
            &unroll.trial,
            &slp.trial,
            &unroll.plan,
            &unroll.charge,
            &slp.plan,
            &slp.charge,
            combined_cost,
            &combined_charge,
        ),
        Ok(())
    );
    let mut forged_cost = combined_cost;
    forged_cost.total = forged_cost.total.saturating_add(1);
    assert!(
        check_unroll_slp_trial_independently(
            &pre,
            &unroll.trial,
            &slp.trial,
            &unroll.plan,
            &unroll.charge,
            &slp.plan,
            &slp.charge,
            forged_cost,
            &combined_charge,
        )
        .is_err()
    );

    let mut committed = pre.clone();
    let mut audit = KirOptimizationAuditState::for_module(pre.module());
    let intermediate = unroll.trial.clone();
    let unroll_plan = unroll.plan.clone();
    let unroll_charge = unroll.charge.clone();
    let slp_plan = slp.plan.clone();
    let slp_charge = slp.charge.clone();
    let committed_cost = combined_cost;
    let committed_charge = combined_charge.clone();
    let final_state = slp.trial.clone();
    let outcome = execute_verified_transaction(
        &mut committed,
        &mut audit,
        unroll_candidate.key.clone(),
        combined_charge.clone(),
        move |trial| {
            *trial = final_state;
            Ok(())
        },
        move |before, after| {
            check_unroll_slp_trial_independently(
                before,
                &intermediate,
                after,
                &unroll_plan,
                &unroll_charge,
                &slp_plan,
                &slp_charge,
                committed_cost,
                &committed_charge,
            )
        },
    );
    assert_eq!(outcome, TransactionOutcome::Committed);
    assert!(!committed.module().functions[0].vector_regions.is_empty());

    let mut rolled_back = pre.clone();
    let before = rolled_back.clone();
    let mut rejected_audit = KirOptimizationAuditState::for_module(pre.module());
    let intermediate = unroll.trial;
    let unroll_plan = unroll.plan;
    let unroll_charge = unroll.charge;
    let mut forged_slp = slp.plan;
    forged_slp.cost.total = forged_slp.cost.total.saturating_add(1);
    let slp_charge = slp.charge;
    let rejected_cost = combined_cost;
    let rejected_charge = combined_charge.clone();
    let final_state = slp.trial;
    let outcome = execute_verified_transaction(
        &mut rolled_back,
        &mut rejected_audit,
        unroll_candidate.key,
        combined_charge,
        move |trial| {
            *trial = final_state;
            Ok(())
        },
        move |before, after| {
            check_unroll_slp_trial_independently(
                before,
                &intermediate,
                after,
                &unroll_plan,
                &unroll_charge,
                &forged_slp,
                &slp_charge,
                rejected_cost,
                &rejected_charge,
            )
        },
    );
    assert!(matches!(outcome, TransactionOutcome::CompilerError(_)));
    assert_eq!(rolled_back, before);
    assert_eq!(rejected_audit.attempts().len(), 1);
    let budget = rejected_audit
        .ledger()
        .budget(unroll_candidate.function)
        .unwrap();
    assert!(budget.proposer_remaining < budget.proposer_initial);

    let production = state(source);
    let result = run_kir_pass_pipeline(production.module().clone(), KirOptimizationLevel::O3, None);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(result.stats.full_unrolled_loops, 1);
    assert_eq!(result.stats.slp_packs, 1);
    let accepted = result
        .audit
        .attempts()
        .iter()
        .filter(|attempt| attempt.disposition == CandidateDisposition::Accepted)
        .collect::<Vec<_>>();
    assert_eq!(accepted.len(), 1, "{accepted:#?}");
    assert!(
        matches!(
            accepted[0].key,
            CandidateKey::LoopFrontier {
                kind: calckernel::LoopCandidateKind::FullUnroll,
                variant: calckernel::LoopCandidateVariant::Slp,
                ..
            }
        ),
        "production did not commit unroll+SLP as one atomic winner: {accepted:#?}"
    );
}
