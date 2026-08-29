use calckernel::{
    ContractFactSet, ContractInstanceSource, FactScope, KirBoundsMode, KirBuildConfig, KirConsumer,
    KirInstructionKind, KirOptimizationLevel, KirOverflowMode, KirSanitizerMode, SourceFile,
    build_kir_module, check, import_contract_facts, lower_to_mir, print_kir_module,
    run_kir_pass_pipeline, validate_kir_optimization_evidence,
};

fn build(
    source_text: &str,
    overflow_mode: KirOverflowMode,
) -> (calckernel::KirModule, Option<ContractFactSet>) {
    let checked = check(&SourceFile::new("o2.ck", source_text));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR");
    let kir = build_kir_module(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::Inspection,
            overflow_mode,
            bounds_mode: KirBoundsMode::Checked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
    )
    .expect("KIR");
    let contracts = checked
        .checked_program
        .functions
        .iter()
        .any(|function| function.is_unsafe)
        .then(|| {
            import_contract_facts(&kir, &checked.checked_program, 0).expect("contract import")
        });
    (kir, contracts)
}

#[test]
fn kir_o2_pipeline_should_use_the_exact_verified_pass_order() {
    let (kir, contracts) = build(
        "export fn answer() -> i32 { return 20 + 22; }",
        KirOverflowMode::Checked,
    );
    let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O2, contracts.as_ref());

    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(
        result
            .records
            .iter()
            .map(|record| record.name.as_str())
            .collect::<Vec<_>>(),
        vec![
            "cfg-canonicalize",
            "sccp-range",
            "check-elimination",
            "effect-aware-inline",
            "memory-ssa-refine",
            "gvn",
            "load-forwarding",
            "dead-store-elimination",
            "sccp-range-post-inline",
            "check-elimination-post-inline",
            "dead-code-elimination",
            "cleanup",
        ]
    );
    assert!(result.records.iter().all(|record| record.verified));
}

#[test]
fn kir_o2_inline_should_clone_small_value_void_and_multi_return_callees() {
    let (kir, contracts) = build(
        r#"
        fn plus_one(n: i32) -> i32 { return n + 1; }
        fn choose(flag: bool) -> i32 { if flag { return 40; } return 41; }
        fn say() -> void { print_i32(7); }
        export fn run(flag: bool) -> i32 {
          say();
          return plus_one(choose(flag));
        }
        "#,
        KirOverflowMode::Checked,
    );
    let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O2, contracts.as_ref());
    let caller = result
        .artifact
        .as_ref()
        .expect("artifact")
        .functions
        .iter()
        .find(|function| function.name == "run")
        .expect("run");

    assert!(
        !caller
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .any(|instruction| { matches!(instruction.kind, KirInstructionKind::Call { .. }) })
    );
    assert_eq!(
        caller
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .filter(|instruction| matches!(
                instruction.kind,
                KirInstructionKind::RuntimeCall { .. }
            ))
            .count(),
        1
    );
    assert_eq!(result.stats.inlined_calls, 3);
}

#[test]
fn kir_o2_inline_should_respect_size_budget_and_reprove_cloned_checked_guards() {
    let (safe_kir, safe_contracts) = build(
        r#"
        fn plus_one(n: i32) -> i32 { return n + 1; }
        export fn run() -> i32 { return plus_one(41); }
        "#,
        KirOverflowMode::Checked,
    );
    let safe = run_kir_pass_pipeline(safe_kir, KirOptimizationLevel::O2, safe_contracts.as_ref());
    let run = safe
        .artifact
        .as_ref()
        .expect("artifact")
        .functions
        .iter()
        .find(|function| function.name == "run")
        .expect("run");
    assert!(
        !run.blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .any(|instruction| matches!(instruction.kind, KirInstructionKind::Guard { .. }))
    );
    assert!(
        safe.eliminated_guards
            .iter()
            .any(|elimination| { elimination.function == run.id && elimination.proof.is_some() })
    );

    let (large_kir, large_contracts) = build(
        r#"
        fn large(n: i32) -> i32 {
          let a0: i32 = n + 1; let a1: i32 = a0 + 1;
          let a2: i32 = a1 + 1; let a3: i32 = a2 + 1;
          let a4: i32 = a3 + 1; let a5: i32 = a4 + 1;
          let a6: i32 = a5 + 1; let a7: i32 = a6 + 1;
          let a8: i32 = a7 + 1; let a9: i32 = a8 + 1;
          let a10: i32 = a9 + 1; let a11: i32 = a10 + 1;
          return a11;
        }
        export fn keep_call(n: i32) -> i32 { return large(n); }
        "#,
        KirOverflowMode::Checked,
    );
    let large = run_kir_pass_pipeline(
        large_kir,
        KirOptimizationLevel::O2,
        large_contracts.as_ref(),
    );
    let keep = large
        .artifact
        .as_ref()
        .expect("large artifact")
        .functions
        .iter()
        .find(|function| function.name == "keep_call")
        .expect("keep_call");
    assert!(
        keep.blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .any(|instruction| matches!(instruction.kind, KirInstructionKind::Call { .. }))
    );
}

#[test]
fn unsafe_inline_contract_facts_should_be_scoped_only_to_fresh_clone_blocks() {
    let (kir, contracts) = build(
        r#"
        unsafe fn bounded(n: u32) -> u32
        contract { requires n < 8; }
        { return n; }

        export unsafe fn run(n: u32) -> u32
        contract { requires n < 8; }
        { unsafe { return bounded(n); } }
        "#,
        KirOverflowMode::Checked,
    );
    let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O2, contracts.as_ref());
    let imported = result.contract_facts.as_ref().expect("updated facts");
    let clone = imported
        .instances()
        .iter()
        .find(|instance| matches!(instance.source, ContractInstanceSource::InlineClone { .. }))
        .expect("inline clone instance");

    assert!(!clone.facts.is_empty());
    assert!(result.stats.inlined_calls >= 1);
}

#[test]
fn unsafe_inline_scope_mutation_should_be_rejected_by_the_independent_checker() {
    let (kir, contracts) = build(
        r#"
        unsafe fn bounded(n: u32) -> u32
        contract { requires n < 8; }
        { return n; }

        export unsafe fn run(n: u32) -> u32
        contract { requires n < 8; }
        { unsafe { return bounded(n); } }
        "#,
        KirOverflowMode::Checked,
    );
    let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O2, contracts.as_ref());
    let artifact = result.artifact.as_ref().expect("artifact");
    let caller = artifact
        .functions
        .iter()
        .find(|function| function.name == "run")
        .expect("caller");
    let mut mutated = result.contract_facts.clone().expect("updated facts");
    let clone = mutated
        .instances()
        .iter()
        .find(|instance| matches!(instance.source, ContractInstanceSource::InlineClone { .. }))
        .expect("clone")
        .clone();
    mutated
        .facts_mut()
        .get_mut(clone.facts[0])
        .expect("clone fact")
        .scope = FactScope::FunctionEntry(caller.id);

    let evidence = validate_kir_optimization_evidence(
        artifact,
        Some(&mutated),
        &result.proofs,
        &result.eliminated_guards,
        0,
    );
    assert!(evidence.errors.iter().any(|error| {
        error
            .message
            .contains("scope does not match contract instance")
    }));
}

#[test]
fn gvn_strict_f64_should_merge_only_exact_dominating_expression_keys() {
    let (kir, contracts) = build(
        r#"
        export fn calc(a: f64, b: f64) -> f64 {
          let x: f64 = a + b;
          let y: f64 = a + b;
          return x + y;
        }
        "#,
        KirOverflowMode::Checked,
    );
    let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O2, contracts.as_ref());
    let function = &result.artifact.as_ref().expect("artifact").functions[0];
    let binary_count = function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .filter(|instruction| matches!(instruction.kind, KirInstructionKind::Binary { .. }))
        .count();

    assert_eq!(binary_count, 2);
    assert_eq!(result.stats.gvn_rewrites, 1);
}

#[test]
fn gvn_near_neighbors_should_not_cross_operand_or_checked_failure_boundaries() {
    let (strict_kir, strict_contracts) = build(
        r#"
        export fn strict(a: f64, b: f64) -> f64 {
          let x: f64 = a + b;
          let y: f64 = b + a;
          return x + y;
        }
        "#,
        KirOverflowMode::Checked,
    );
    let strict = run_kir_pass_pipeline(
        strict_kir,
        KirOptimizationLevel::O2,
        strict_contracts.as_ref(),
    );
    assert_eq!(
        strict.artifact.as_ref().expect("strict artifact").functions[0]
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .filter(|instruction| matches!(instruction.kind, KirInstructionKind::Binary { .. }))
            .count(),
        3
    );

    let (checked_kir, checked_contracts) = build(
        r#"
        export fn checked(a: i32, b: i32) -> i32 {
          let x: i32 = a + b;
          let y: i32 = a + b;
          return x + y;
        }
        "#,
        KirOverflowMode::Checked,
    );
    let checked = run_kir_pass_pipeline(
        checked_kir,
        KirOptimizationLevel::O2,
        checked_contracts.as_ref(),
    );
    assert_eq!(
        checked
            .artifact
            .as_ref()
            .expect("checked artifact")
            .functions[0]
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .filter(|instruction| matches!(instruction.kind, KirInstructionKind::Binary { .. }))
            .count(),
        3
    );
}

#[test]
fn gvn_dominating_expression_should_reach_child_but_not_require_sibling_guessing() {
    let (kir, contracts) = build(
        r#"
        export fn dominated(a: f64, b: f64, flag: bool) -> f64 {
          let x: f64 = a + b;
          if flag { return a + b; }
          return x;
        }
        "#,
        KirOverflowMode::Checked,
    );
    let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O2, contracts.as_ref());
    let function = &result.artifact.as_ref().expect("artifact").functions[0];
    let binary_count = function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .filter(|instruction| matches!(instruction.kind, KirInstructionKind::Binary { .. }))
        .count();
    assert_eq!(
        binary_count,
        1,
        "{}",
        print_kir_module(result.artifact.as_ref().expect("artifact"))
    );
}

#[test]
fn memory_opt_should_forward_same_version_load_and_eliminate_overwritten_store() {
    let (kir, contracts) = build(
        r#"
        export unsafe fn sum_twice(items: slice<i32>, n: u32) -> i32
        contract { requires n < items.len; effects read(items); }
        { let a: i32 = items[n]; let b: i32 = items[n]; return a + b; }

        export fn overwrite(out: ptr<i32>) -> void {
          out[0] = 1;
          out[0] = 2;
        }
        "#,
        KirOverflowMode::Checked,
    );
    let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O2, contracts.as_ref());
    let artifact = result.artifact.as_ref().expect("artifact");
    let sum = artifact
        .functions
        .iter()
        .find(|function| function.name == "sum_twice")
        .expect("sum");
    let overwrite = artifact
        .functions
        .iter()
        .find(|function| function.name == "overwrite")
        .expect("overwrite");

    assert_eq!(
        sum.blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .filter(|instruction| { matches!(instruction.kind, KirInstructionKind::Load { .. }) })
            .count(),
        1
    );
    assert_eq!(
        overwrite
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .filter(|instruction| { matches!(instruction.kind, KirInstructionKind::Store { .. }) })
            .count(),
        1
    );
    assert_eq!(result.stats.forwarded_loads, 1);
    assert_eq!(result.stats.eliminated_stores, 1);
}

#[test]
fn memory_opt_third_root_call_join_and_effect_barriers_should_block_rewrites() {
    let (kir, contracts) = build(
        r#"
        fn write_one(items: slice<i32>, n: u32) -> void { items[n] = 1; }

        export unsafe fn third_root(a: slice<i32>, b: slice<i32>, c: slice<i32>, n: u32) -> i32
        contract {
          requires noalias(a, b);
          requires n < a.len;
          requires n < b.len;
          requires n < c.len;
          effects read(a), write(c);
        }
        { let x: i32 = a[n]; c[n] = 1; let y: i32 = a[n]; return x + y; }

        export unsafe fn call_barrier(items: slice<i32>, n: u32) -> i32
        contract { requires n < items.len; effects readwrite(items); }
        {
          let x: i32 = items[n];
          write_one(items, n);
          let y: i32 = items[n];
          return x + y;
        }

        export unsafe fn join_barrier(items: slice<i32>, n: u32, flag: bool) -> i32
        contract { requires n < items.len; effects readwrite(items); }
        {
          let x: i32 = items[n];
          if flag { items[n] = 1; }
          let y: i32 = items[n];
          return x + y;
        }

        export fn effect_barrier(out: ptr<i32>, n: i32) -> void {
          out[0] = 1;
          print_i32(n + 1);
          out[0] = 2;
        }
        "#,
        KirOverflowMode::Checked,
    );
    let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O2, contracts.as_ref());
    let artifact = result.artifact.as_ref().expect("artifact");
    for name in ["third_root", "call_barrier", "join_barrier"] {
        let function = artifact
            .functions
            .iter()
            .find(|function| function.name == name)
            .expect(name);
        assert_eq!(
            function
                .blocks
                .iter()
                .flat_map(|block| &block.instructions)
                .filter(|instruction| matches!(instruction.kind, KirInstructionKind::Load { .. }))
                .count(),
            2,
            "{name}"
        );
    }
    let effect = artifact
        .functions
        .iter()
        .find(|function| function.name == "effect_barrier")
        .expect("effect barrier");
    assert_eq!(
        effect
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .filter(|instruction| matches!(instruction.kind, KirInstructionKind::Store { .. }))
            .count(),
        2
    );
}

#[test]
fn runtime_print_o2_inline_should_preserve_exact_observable_count_and_order() {
    let (kir, contracts) = build(
        r#"
        fn emit(n: i32) -> void { print_i32(n); }
        fn main() -> void { emit(1); emit(2); }
        "#,
        KirOverflowMode::Checked,
    );
    let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O2, contracts.as_ref());
    let main = result
        .artifact
        .as_ref()
        .expect("artifact")
        .functions
        .iter()
        .find(|function| function.name == "main")
        .expect("main");
    let effects = main
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .filter(|instruction| matches!(instruction.kind, KirInstructionKind::RuntimeCall { .. }))
        .map(|instruction| instruction.effect.as_ref().expect("runtime effect").order)
        .collect::<Vec<_>>();
    assert_eq!(effects, vec![0, 1]);
}
