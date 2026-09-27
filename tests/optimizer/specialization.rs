use std::{fs, process::Command, time::SystemTime};

use calckernel::{
    CandidateDisposition, CandidateKey, ContractFactSet, ContractInstanceSource, KirBoundsMode,
    KirBuildConfig, KirConsumer, KirOptimizationLevel, KirOverflowMode, KirSanitizerMode,
    KirVerifiedProgramState, ProofId, SourceFile, SpecializationFactSource, build_kir_module,
    check, check_specialization_plan_independently, discover_specialization_candidates,
    emit_c_kir_header, emit_c_kir_module_with_contracts, import_contract_facts, kir_function_units,
    lower_to_mir, prepare_specialization_trial, run_kir_pass_pipeline,
    specialization_profitability_threshold,
};

fn build(
    source: &str,
    sanitizer_mode: KirSanitizerMode,
) -> (calckernel::KirModule, Option<ContractFactSet>) {
    build_for_consumer(source, sanitizer_mode, KirConsumer::Inspection)
}

fn build_for_consumer(
    source: &str,
    sanitizer_mode: KirSanitizerMode,
    consumer: KirConsumer,
) -> (calckernel::KirModule, Option<ContractFactSet>) {
    let checked = check(&SourceFile::new("specialization.ck", source));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR");
    let module = build_kir_module(
        &mir,
        KirBuildConfig {
            consumer: if sanitizer_mode == KirSanitizerMode::Contracts {
                KirConsumer::NativeExecutable
            } else {
                consumer
            },
            overflow_mode: KirOverflowMode::Checked,
            bounds_mode: KirBoundsMode::Checked,
            sanitizer_mode,
        },
    )
    .expect("KIR");
    let contracts = checked
        .checked_program
        .functions
        .iter()
        .any(|function| function.is_unsafe)
        .then(|| {
            import_contract_facts(&module, &checked.checked_program, 0).expect("contract facts")
        });
    (module, contracts)
}

const PROFITABLE: &str = r#"
fn choose(n: i32) -> i32 {
  if n == 7 { return 40; }
  if n == 8 { return 41; }
  return n + 100;
}
export fn answer() -> i32 { return choose(7) + 2; }
"#;

#[test]
fn specialization_candidate_discovery_should_use_a_canonical_fact_digest_and_total_key_order() {
    let (module, contracts) = build(PROFITABLE, KirSanitizerMode::Disabled);
    let first = discover_specialization_candidates(&module, contracts.as_ref());
    assert_eq!(first.candidates.len(), 1, "{first:?}");
    let candidate = &first.candidates[0];
    assert_eq!(candidate.facts.len(), 1);
    assert_eq!(candidate.fact_set_digest.len(), 64);
    assert!(matches!(candidate.key, CandidateKey::Specialization { .. }));

    let mut reordered = module;
    reordered.functions.reverse();
    let second = discover_specialization_candidates(&reordered, contracts.as_ref());
    assert_eq!(first.candidates, second.candidates);
}

#[test]
fn specialization_should_commit_real_scalar_simplification_and_retain_generic_body() {
    let (module, contracts) = build(PROFITABLE, KirSanitizerMode::Disabled);
    let generic_before = module
        .functions
        .iter()
        .find(|function| function.name == "choose")
        .unwrap()
        .clone();
    let result = run_kir_pass_pipeline(module, KirOptimizationLevel::O3, contracts.as_ref());
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let artifact = result.artifact.as_ref().expect("artifact");
    let generic = artifact
        .functions
        .iter()
        .find(|function| function.name == "choose")
        .expect("generic retained");
    let specialized = artifact
        .functions
        .iter()
        .find(|function| function.name.starts_with("__ck_spec_choose_"))
        .expect("accepted deterministic clone");
    assert!(!specialized.exported);
    assert_eq!(generic.params, generic_before.params);
    assert_eq!(generic.return_type, generic_before.return_type);
    assert!(
        kir_function_units(specialized) + 2 <= kir_function_units(generic),
        "specialization must materialize at least two cost units of scalar saving"
    );
    assert_eq!(result.stats.specialized_clones, 1);
    assert!(result.audit.attempts().iter().any(|attempt| {
        attempt.disposition == CandidateDisposition::Accepted
            && matches!(attempt.key, CandidateKey::Specialization { .. })
    }));
}

#[test]
fn specialization_scope_should_bind_each_trusted_fact_to_its_exact_call_instance() {
    let source = r#"
unsafe fn bounded(n: u32) -> u32 contract { requires n == 7; } {
  if n == 7 { return 1; }
  return 0;
}
export fn both(a: u32, b: u32) -> u32 {
  let x: u32 = 0;
  unsafe { x = bounded(a); x = x + bounded(b); }
  return x;
}
"#;
    let (module, contracts) = build(source, KirSanitizerMode::Disabled);
    let contracts = contracts.expect("contracts");
    let discovery = discover_specialization_candidates(&module, Some(&contracts));
    assert_eq!(discovery.candidates.len(), 2, "{discovery:?}");
    for candidate in &discovery.candidates {
        let call = match candidate.key {
            CandidateKey::Specialization { call, .. } => call,
            _ => unreachable!(),
        };
        let instances = candidate
            .facts
            .iter()
            .filter_map(|fact| match fact.source {
                SpecializationFactSource::TrustedContract { instance, .. } => Some(instance),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(!instances.is_empty());
        assert!(instances.iter().all(|instance| {
            contracts.instances().iter().any(|record| {
                record.id == *instance
                    && matches!(
                        record.source,
                        ContractInstanceSource::Call { instruction, .. } if instruction == call
                    )
            })
        }));
    }
    let result = run_kir_pass_pipeline(module, KirOptimizationLevel::O3, Some(&contracts));
    assert!(result.errors.is_empty(), "{:?}", result.errors);
}

#[test]
fn specialization_scope_should_exclude_exported_recursive_and_clone_roots() {
    for source in [
        "export fn public(n: i32) -> i32 { return n + 1; } export fn caller() -> i32 { return public(7); }",
        "fn recursive(n: u32) -> u32 { if n == 0 { return 0; } return recursive(n - 1); } export fn caller() -> u32 { return recursive(7); }",
        "fn __ck_spec_old_deadbeef(n: i32) -> i32 { return n + 1; } export fn caller() -> i32 { return __ck_spec_old_deadbeef(7); }",
    ] {
        let (module, contracts) = build(source, KirSanitizerMode::Disabled);
        assert!(
            discover_specialization_candidates(&module, contracts.as_ref())
                .candidates
                .is_empty(),
            "{source}"
        );
    }
}

#[test]
fn specialization_budget_should_debit_caller_and_callee_even_when_profitability_rejects() {
    let source = "fn identity(n: i32) -> i32 { return n; } export fn caller() -> i32 { return identity(7); }";
    let (module, contracts) = build(source, KirSanitizerMode::Disabled);
    let caller = module
        .functions
        .iter()
        .find(|f| f.name == "caller")
        .unwrap()
        .id;
    let callee = module
        .functions
        .iter()
        .find(|f| f.name == "identity")
        .unwrap()
        .id;
    let result = run_kir_pass_pipeline(module, KirOptimizationLevel::O3, contracts.as_ref());
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert!(
        result
            .module
            .functions
            .iter()
            .all(|function| !function.name.starts_with("__ck_spec_identity_"))
    );
    let attempt = result
        .audit
        .attempts()
        .iter()
        .find(|attempt| matches!(attempt.key, CandidateKey::Specialization { .. }))
        .expect("rejected attempt");
    assert_eq!(attempt.disposition, CandidateDisposition::Rejected);
    for function in [caller, callee] {
        let budget = result.audit.ledger().budget(function).expect("budget");
        assert!(budget.proposer_remaining < budget.proposer_initial);
        assert!(budget.checker_remaining < budget.checker_initial);
    }
}

#[test]
fn specialization_abi_should_keep_exported_and_generic_symbols_stable() {
    let (module, contracts) =
        build_for_consumer(PROFITABLE, KirSanitizerMode::Disabled, KirConsumer::C);
    let header_before = emit_c_kir_header(&module);
    let exported = module
        .functions
        .iter()
        .filter(|function| function.exported)
        .map(|function| {
            (
                function.name.clone(),
                function.params.clone(),
                function.return_type.clone(),
            )
        })
        .collect::<Vec<_>>();
    let result = run_kir_pass_pipeline(module, KirOptimizationLevel::O3, contracts.as_ref());
    let artifact = result.artifact.expect("artifact");
    let after = artifact
        .functions
        .iter()
        .filter(|function| function.exported)
        .map(|function| {
            (
                function.name.clone(),
                function.params.clone(),
                function.return_type.clone(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(after, exported);
    assert!(
        artifact
            .functions
            .iter()
            .any(|function| function.name == "choose")
    );
    assert!(
        artifact
            .functions
            .iter()
            .filter(|function| function.exported)
            .all(|function| { !function.name.starts_with("__ck_spec_") })
    );

    let header_after = emit_c_kir_header(&artifact);
    assert_eq!(header_after, header_before, "public C header ABI changed");
    assert!(!header_after.contains("choose"), "{header_after}");
    assert!(!header_after.contains("__ck_spec_"), "{header_after}");

    let c = emit_c_kir_module_with_contracts(&artifact, contracts.as_ref())
        .expect("specialized C artifact");
    assert!(c.contains("static CKC_UNUSED CK_Status choose("), "{c}");
    assert!(
        c.contains("static CKC_UNUSED CK_Status __ck_spec_choose_f"),
        "{c}"
    );

    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("ckc-specialization-abi-{unique}"));
    fs::create_dir(&directory).expect("create ABI audit directory");
    let source = directory.join("specialized.c");
    let object = directory.join("specialized.o");
    fs::write(&source, c).expect("write specialized C artifact");
    let compile = Command::new("clang")
        .args(["-std=c11", "-Wall", "-Wextra", "-Werror", "-c"])
        .arg(&source)
        .arg("-o")
        .arg(&object)
        .output()
        .expect("run clang ABI audit");
    assert!(
        compile.status.success(),
        "clang ABI audit failed: {}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let symbols = Command::new("nm")
        .arg("-g")
        .arg(&object)
        .output()
        .expect("run nm ABI audit");
    assert!(symbols.status.success(), "nm ABI audit failed");
    let symbols = String::from_utf8_lossy(&symbols.stdout);
    let names = symbols
        .lines()
        .filter_map(|line| line.split_whitespace().last())
        .map(|name| name.trim_start_matches('_'))
        .collect::<Vec<_>>();
    assert!(names.contains(&"answer"), "{symbols}");
    assert!(
        !names.iter().any(|name| name.contains("choose")),
        "{symbols}"
    );
    assert!(
        !names.iter().any(|name| name.contains("__ck_spec_")),
        "{symbols}"
    );
    fs::remove_dir_all(&directory).expect("remove ABI audit directory");
}

#[test]
fn sanitizer_specialization_should_be_disabled_with_a_stable_reason() {
    let source = r#"
fn choose(n: i32) -> i32 {
  if n == 7 { return 40; }
  if n == 8 { return 41; }
  return n + 100;
}
fn main() -> i32 { return choose(7); }
"#;
    let (module, contracts) = build(source, KirSanitizerMode::Contracts);
    let result = run_kir_pass_pipeline(module, KirOptimizationLevel::O3, contracts.as_ref());
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert!(result.audit.attempts().is_empty());
    assert!(result.analysis_fallbacks.iter().any(|fallback| {
        fallback.pass == "specialization" && fallback.reason == "sanitizer-mode-disabled"
    }));
    let record = result
        .records
        .iter()
        .find(|record| record.name == "specialization-frontier")
        .unwrap();
    assert!(!record.changed);
}

#[test]
fn specialization_fixed_slice_length_should_materialize_constant_bounds_before_loop_stages() {
    let source = r#"
fn bounded(items: slice<u32>) -> u32 {
  if items.len == 8 { return items[7]; }
  return 0;
}
export fn kernel(data: ptr<u32>) -> u32 { return bounded(slice(data, 8)); }
"#;
    let (module, contracts) = build(source, KirSanitizerMode::Disabled);
    let result = run_kir_pass_pipeline(module, KirOptimizationLevel::O3, contracts.as_ref());
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert!(result.stats.specialized_clones >= 1);
    let clone = result
        .artifact
        .as_ref()
        .unwrap()
        .functions
        .iter()
        .find(|function| function.name.starts_with("__ck_spec_bounded_"))
        .expect("slice-length clone");
    assert!(
        !clone
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .any(|instruction| {
                matches!(
                    instruction.kind,
                    calckernel::KirInstructionKind::SliceLen { .. }
                )
            })
    );
}

#[test]
fn specialization_checker_should_reject_mapping_scope_digest_cost_growth_proof_and_budget_mutations()
 {
    let (module, contracts) = build(PROFITABLE, KirSanitizerMode::Disabled);
    let state = KirVerifiedProgramState::new(module, contracts, 0).expect("verified pre-state");
    let candidate = discover_specialization_candidates(state.module(), state.contract_facts())
        .candidates
        .remove(0);
    let prepared = prepare_specialization_trial(&state, &candidate, 0).expect("proposal");
    assert_eq!(
        check_specialization_plan_independently(
            &state,
            &prepared.trial,
            &prepared.plan,
            &prepared.charge,
        ),
        Ok(())
    );

    let mut mapping = prepared.plan.clone();
    mapping.mapping.parameters[0].1 = mapping.mapping.parameters[0].0;
    assert!(
        check_specialization_plan_independently(
            &state,
            &prepared.trial,
            &mapping,
            &prepared.charge,
        )
        .is_err()
    );

    let mut scope = prepared.plan.clone();
    scope.facts[0].source = SpecializationFactSource::TrustedContract {
        instance: calckernel::ContractInstanceId::from_index(99),
        fact: calckernel::FactId::from_index(99),
    };
    assert!(
        check_specialization_plan_independently(&state, &prepared.trial, &scope, &prepared.charge,)
            .is_err()
    );

    let mut digest = prepared.plan.clone();
    digest.fact_set_digest = "00".repeat(32);
    assert!(check_specialization_plan_independently(
        &state,
        &prepared.trial,
        &digest,
        &prepared.charge,
    )
    .is_err());

    let mut cost = prepared.plan.clone();
    cost.cost.total = cost.cost.total.saturating_add(1);
    assert!(
        check_specialization_plan_independently(&state, &prepared.trial, &cost, &prepared.charge,)
            .is_err()
    );

    let mut growth = prepared.plan.clone();
    growth.growth.module_after_units = growth.growth.module_after_units.saturating_add(1);
    assert!(check_specialization_plan_independently(
        &state,
        &prepared.trial,
        &growth,
        &prepared.charge,
    )
    .is_err());

    let mut proof = prepared.plan.clone();
    proof.fact_scope_proof = ProofId::from_index(u32::MAX);
    assert!(
        check_specialization_plan_independently(&state, &prepared.trial, &proof, &prepared.charge,)
            .is_err()
    );

    let mut charge = prepared.charge.clone();
    charge.checker_units = charge.checker_units.saturating_add(1);
    assert!(
        check_specialization_plan_independently(&state, &prepared.trial, &prepared.plan, &charge,)
            .is_err()
    );

    let mut forged_trial = prepared.trial;
    forged_trial
        .module_mut()
        .functions
        .iter_mut()
        .find(|function| function.id == prepared.plan.clone)
        .unwrap()
        .exported = true;
    assert!(
        check_specialization_plan_independently(
            &state,
            &forged_trial,
            &prepared.plan,
            &prepared.charge,
        )
        .is_err()
    );
}

#[test]
fn specialization_budget_threshold_should_accept_exact_ten_percent_and_two_units_only() {
    assert!(specialization_profitability_threshold(20, 18));
    assert!(specialization_profitability_threshold(19, 17));
    assert!(!specialization_profitability_threshold(20, 19));
    assert!(!specialization_profitability_threshold(19, 18));
    assert!(!specialization_profitability_threshold(21, 19));
}

#[test]
fn specialization_budget_should_reuse_one_clone_without_refunding_either_attempt() {
    let source = r#"
fn choose(n: i32) -> i32 {
  if n == 7 { return 40; }
  if n == 8 { return 41; }
  return n + 100;
}
export fn answer() -> i32 { return choose(7) + choose(7); }
"#;
    let (module, contracts) = build(source, KirSanitizerMode::Disabled);
    let result = run_kir_pass_pipeline(module, KirOptimizationLevel::O3, contracts.as_ref());
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(result.stats.specialized_clones, 1);
    assert_eq!(result.stats.reused_specializations, 1);
    assert_eq!(
        result
            .module
            .functions
            .iter()
            .filter(|function| function.name.starts_with("__ck_spec_choose_"))
            .count(),
        1
    );
    assert_eq!(
        result
            .audit
            .attempts()
            .iter()
            .filter(|attempt| matches!(attempt.key, CandidateKey::Specialization { .. }))
            .map(|attempt| attempt.disposition)
            .collect::<Vec<_>>(),
        vec![CandidateDisposition::Accepted, CandidateDisposition::Reused]
    );
}

#[test]
fn specialization_budget_should_cap_each_original_at_three_distinct_clones() {
    let source = r#"
fn choose(n: i32) -> i32 {
  if n == 7 { return 40; }
  if n == 8 { return 41; }
  if n == 9 { return 42; }
  if n == 10 { return 43; }
  return n + 100;
}
export fn answer() -> i32 { return choose(7) + choose(8) + choose(9) + choose(10); }
"#;
    let (module, contracts) = build(source, KirSanitizerMode::Disabled);
    let result = run_kir_pass_pipeline(module, KirOptimizationLevel::O3, contracts.as_ref());
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(result.stats.specialized_clones, 3);
    assert_eq!(result.stats.specialization_limit_fallbacks, 1);
    assert_eq!(
        result
            .module
            .functions
            .iter()
            .filter(|function| function.name.starts_with("__ck_spec_choose_"))
            .count(),
        3
    );
}
