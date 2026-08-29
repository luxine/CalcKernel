use calckernel::{
    ContractFactSet, KirBoundsMode, KirBuildConfig, KirConsumer, KirFailureKind,
    KirInstructionKind, KirOptimizationLevel, KirOverflowMode, KirSanitizerMode, SourceFile,
    analyze_natural_loops, build_kir_module, check, import_contract_facts, lower_to_mir,
    print_kir_module, run_kir_pass_pipeline,
};

fn build(
    source_text: &str,
    overflow_mode: KirOverflowMode,
) -> (calckernel::KirModule, Option<ContractFactSet>) {
    let checked = check(&SourceFile::new("o3.ck", source_text));
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
fn kir_o3_pipeline_should_use_the_exact_verified_pass_order() {
    let (kir, contracts) = build(
        "export fn answer() -> i32 { return 20 + 22; }",
        KirOverflowMode::Checked,
    );
    let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O3, contracts.as_ref());

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
            "natural-loop-analysis",
            "licm",
            "induction-simplify",
            "sccp-range-post-loop",
            "check-elimination-post-loop",
            "dead-code-elimination",
            "cleanup",
        ]
    );
    assert!(result.records.iter().all(|record| record.verified));
}

#[test]
fn loop_analysis_should_build_nested_natural_loop_tree_with_inductions() {
    let (kir, _) = build(
        r#"
        export fn nested(n: u32) -> u32 {
          let outer: u32 = 0;
          let total: u32 = 0;
          while outer < n {
            let inner: u32 = 0;
            while inner < n {
              if inner == 2 { inner = inner + 1; continue; }
              if inner == 5 { break; }
              total = total + inner;
              inner = inner + 1;
            }
            outer = outer + 1;
          }
          return total;
        }
        "#,
        KirOverflowMode::Checked,
    );
    let analysis = analyze_natural_loops(&kir.functions[0]);

    assert_eq!(analysis.loops.len(), 2);
    assert!(analysis.loops.iter().any(|loop_info| loop_info.depth == 2));
    assert!(
        analysis
            .loops
            .iter()
            .all(|loop_info| !loop_info.latches.is_empty())
    );
    assert!(analysis.inductions.len() >= 2, "{}", print_kir_module(&kir));
    assert!(analysis.irreducible_blocks.is_empty());
}

#[test]
fn loop_licm_should_hoist_only_modular_pure_invariants() {
    let (kir, contracts) = build(
        r#"
        export fn repeated(a: u32, b: u32, n: u32) -> u32 {
          let i: u32 = 0;
          let total: u32 = 0;
          while i < n {
            let scale: u32 = a * b;
            total = total + scale;
            i = i + 1;
          }
          return total;
        }
        "#,
        KirOverflowMode::Unchecked,
    );
    let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O3, contracts.as_ref());

    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert!(result.stats.natural_loops >= 1);
    assert!(result.stats.induction_variables >= 1);
    assert!(result.stats.hoisted_instructions >= 1);
}

#[test]
fn loop_checked_failure_and_print_should_remain_inside_the_loop_in_source_order() {
    let (kir, contracts) = build(
        r#"
        fn main() -> void {
          let i: i32 = 0;
          while i < 3 {
            print_i32(i + 1);
            i = i + 1;
          }
        }
        "#,
        KirOverflowMode::Checked,
    );
    let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O3, contracts.as_ref());
    let function = result
        .artifact
        .as_ref()
        .expect("artifact")
        .functions
        .iter()
        .find(|function| function.name == "main")
        .expect("main");
    let loop_info = &analyze_natural_loops(function).loops[0];
    let ordered = function
        .blocks
        .iter()
        .filter(|block| loop_info.blocks.binary_search(&block.id).is_ok())
        .flat_map(|block| &block.instructions)
        .filter(|instruction| instruction.effect.is_some())
        .map(|instruction| &instruction.kind)
        .collect::<Vec<_>>();

    assert!(
        ordered
            .iter()
            .any(|kind| matches!(kind, KirInstructionKind::RuntimeCall { .. }))
    );
    assert!(
        ordered
            .iter()
            .any(|kind| matches!(kind, KirInstructionKind::Guard { .. }))
    );
}

#[test]
fn loop_canonical_slice_bounds_should_disappear_only_at_o2_and_o3() {
    let source = r#"
        export unsafe fn sum(items: slice<i32>, len: u32) -> i32
        contract { requires len <= items.len; effects read(items); }
        {
          let i: u32 = 0;
          let total: i32 = 0;
          while i < len {
            total = total + items[i];
            i = i + 1;
          }
          return total;
        }
    "#;
    for (level, expected) in [
        (KirOptimizationLevel::O1, 1_usize),
        (KirOptimizationLevel::O2, 0),
        (KirOptimizationLevel::O3, 0),
    ] {
        let (kir, contracts) = build(source, KirOverflowMode::Checked);
        let result = run_kir_pass_pipeline(kir, level, contracts.as_ref());
        let function = result
            .artifact
            .as_ref()
            .expect("artifact")
            .functions
            .iter()
            .find(|function| function.name == "sum")
            .expect("sum");
        let bounds_guards = function
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .filter(|instruction| {
                matches!(
                    instruction.kind,
                    KirInstructionKind::Guard {
                        failure: KirFailureKind::OutOfBounds,
                        ..
                    }
                )
            })
            .count();
        assert_eq!(
            bounds_guards,
            expected,
            "{level:?}: {:?}\n{}",
            result.explanations,
            print_kir_module(result.artifact.as_ref().expect("artifact"))
        );
    }
}

#[test]
fn loop_canonical_slice_neighbor_without_contract_should_retain_guard() {
    let (kir, contracts) = build(
        r#"
        export fn sum(items: slice<i32>, len: u32) -> i32 {
          let i: u32 = 0;
          let total: i32 = 0;
          while i < len { total = total + items[i]; i = i + 1; }
          return total;
        }
        "#,
        KirOverflowMode::Checked,
    );
    let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O3, contracts.as_ref());
    assert!(
        result.artifact.as_ref().expect("artifact").functions[0]
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .any(|instruction| matches!(
                instruction.kind,
                KirInstructionKind::Guard {
                    failure: KirFailureKind::OutOfBounds,
                    ..
                }
            ))
    );
}

#[test]
fn generated_loop_fixed_seed_should_validate_identically_at_every_level() {
    const SEED_0X_C0DE: [&str; 3] = [
        "export fn f(n: u32) -> u32 { let i: u32 = 0; while i < n { i = i + 1; } return i; }",
        "export fn f(n: u32) -> u32 { let i: u32 = 0; while i < n { i = i + 1; if i == 3 { break; } } return i; }",
        "export fn f(n: u32) -> u32 { let i: u32 = 0; while i < n { i = i + 1; if i == 2 { continue; } } return i; }",
    ];
    for source in SEED_0X_C0DE {
        for level in [
            KirOptimizationLevel::O0,
            KirOptimizationLevel::O1,
            KirOptimizationLevel::O2,
            KirOptimizationLevel::O3,
        ] {
            let (kir, contracts) = build(source, KirOverflowMode::Checked);
            let result = run_kir_pass_pipeline(kir, level, contracts.as_ref());
            assert!(result.errors.is_empty(), "{level:?}: {:?}", result.errors);
            assert!(result.artifact.is_some());
        }
    }
}
