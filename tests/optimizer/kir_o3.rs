use calckernel::{
    ContractFactSet, KirBoundsMode, KirBuildConfig, KirConsumer, KirFailureKind,
    KirInstructionKind, KirOptimizationLevel, KirOverflowMode, KirSanitizerMode, SourceFile,
    analyze_natural_loops, build_kir_module, check, import_contract_facts, lower_to_mir,
    print_kir_module, run_kir_pass_pipeline,
};

use crate::generated::fixed_seed_kernel_program;

fn build(
    source_text: &str,
    overflow_mode: KirOverflowMode,
) -> (calckernel::KirModule, Option<ContractFactSet>) {
    build_with_modes(source_text, overflow_mode, KirBoundsMode::Checked)
}

fn build_with_modes(
    source_text: &str,
    overflow_mode: KirOverflowMode,
    bounds_mode: KirBoundsMode,
) -> (calckernel::KirModule, Option<ContractFactSet>) {
    let checked = check(&SourceFile::new("o3.ck", source_text));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR");
    let kir = build_kir_module(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::Inspection,
            overflow_mode,
            bounds_mode,
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
fn unchecked_guard_free_pipeline_should_skip_unconsumed_scalar_analysis() {
    let (kir, contracts) = build_with_modes(
        include_str!("../../examples/applications/dijkstra.ck"),
        KirOverflowMode::Unchecked,
        KirBoundsMode::Unchecked,
    );
    let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O3, contracts.as_ref());

    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(result.stats.scalar_functions_analyzed, 0);
}

#[test]
fn checked_guard_pipeline_should_keep_demanded_scalar_analysis() {
    let (kir, contracts) = build(
        "export fn answer() -> i32 { return 20 + 22; }",
        KirOverflowMode::Checked,
    );
    let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O3, contracts.as_ref());

    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert!(result.stats.scalar_functions_analyzed >= 1);
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
fn loop_induction_should_check_every_latch_and_intervening_assignment() {
    for source in [
        "export fn mixed(n: u32, flag: bool) -> u32 { let i: u32 = 0; while i < n { if flag { i = i + 2; continue; } i = i + 1; } return i; }",
        "export fn replaced(n: u32, flag: bool) -> u32 { let i: u32 = 0; while i < n { if flag { i = 4294967295; } i = i + 1; } return i; }",
    ] {
        let (kir, _) = build(source, KirOverflowMode::Checked);
        let analysis = analyze_natural_loops(&kir.functions[0]);
        assert_eq!(analysis.loops.len(), 1);
        assert!(
            analysis.inductions.is_empty(),
            "a recurrence must cover every real SSA path:\n{}\n{analysis:?}",
            print_kir_module(&kir)
        );
    }
}

fn loop_graph(successors: &[&[usize]]) -> calckernel::KirModule {
    let (mut module, _) = build_with_modes(
        "export fn graph(flag: bool) -> u32 { return 0; }",
        KirOverflowMode::Unchecked,
        KirBoundsMode::Unchecked,
    );
    let function = &mut module.functions[0];
    let template = function.blocks[0].clone();
    function.blocks = successors
        .iter()
        .enumerate()
        .map(|(index, successors)| {
            let mut block = template.clone();
            block.id = calckernel::BlockId::from_index(index as u32);
            block.label = format!("graph{index}");
            if index != 0 {
                block.instructions.clear();
            }
            let edge = |target: usize| calckernel::KirEdge {
                target: calckernel::BlockId::from_index(target as u32),
                args: Vec::new(),
                memory_args: Vec::new(),
            };
            block.terminator = match *successors {
                [] => template.terminator.clone(),
                [target] => calckernel::KirTerminator::Jump {
                    edge: edge(*target),
                },
                [left, right] => calckernel::KirTerminator::Branch {
                    condition: function.params[0].value,
                    then_edge: edge(*left),
                    else_edge: edge(*right),
                },
                _ => panic!("KIR has at most two successors"),
            };
            block
        })
        .collect();
    let validation = calckernel::validate_kir_module(&module);
    assert!(validation.errors.is_empty(), "{:?}", validation.errors);
    module
}

#[test]
fn loop_analysis_should_report_irreducible_cycles_and_discard_natural_loop_candidates() {
    for (graph, expected) in [
        (
            vec![vec![1, 2], vec![3, 4], vec![3], vec![1, 2], vec![]],
            vec![1, 2, 3],
        ),
        // One maximal SCC has a dominating outer header, but its inner cycle
        // still has two entries. A maximal-SCC-only check would miss it.
        (
            vec![
                vec![1],
                vec![2, 3],
                vec![4],
                vec![4],
                vec![2, 5],
                vec![1, 6],
                vec![],
            ],
            vec![2, 4],
        ),
    ] {
        let mut module = loop_graph(&graph.iter().map(Vec::as_slice).collect::<Vec<_>>());
        let expected = expected
            .into_iter()
            .map(calckernel::BlockId::from_index)
            .collect::<Vec<_>>();
        let analysis = analyze_natural_loops(&module.functions[0]);
        assert_eq!(analysis.irreducible_blocks, expected);
        assert!(analysis.loops.is_empty() && analysis.inductions.is_empty());
        module.functions[0].blocks[1..].reverse();
        assert_eq!(analyze_natural_loops(&module.functions[0]), analysis);
    }
}

#[test]
fn loop_self_latch_should_not_include_the_preheader() {
    let module = loop_graph(&[&[1], &[1, 2], &[]]);
    let analysis = analyze_natural_loops(&module.functions[0]);
    assert!(analysis.irreducible_blocks.is_empty());
    assert_eq!(analysis.loops.len(), 1);
    assert_eq!(
        analysis.loops[0].blocks,
        vec![calckernel::BlockId::from_index(1)]
    );
}

#[test]
fn loop_analysis_budget_should_discard_partial_results_deterministically() {
    let (module, _) = build(
        "export fn nested(n: u32) -> u32 { let i: u32 = 0; let sum: u32 = 0; while i < n { let j: u32 = 0; while j < n { sum = sum + j; j = j + 1; } i = i + 1; } return sum; }",
        KirOverflowMode::Checked,
    );
    let function = &module.functions[0];
    let full = analyze_natural_loops(function);
    assert!(!full.budget_exhausted);
    assert_eq!(full.loops.len(), 2);
    let mut saw_exhaustion = false;
    let mut saw_success = false;
    let maximum = calckernel::ScalarAnalysisBudget::for_function(
        function,
        calckernel::ScalarAnalysisConfig::default(),
    )
    .max_steps();
    for limit in (0..=maximum).step_by(7).chain(std::iter::once(maximum)) {
        let config = calckernel::ScalarAnalysisConfig::with_max_steps(limit);
        let result = calckernel::analyze_natural_loops_with_config(function, config);
        assert_eq!(
            result,
            calckernel::analyze_natural_loops_with_config(function, config)
        );
        if result.budget_exhausted {
            saw_exhaustion = true;
            assert!(
                result.loops.is_empty()
                    && result.inductions.is_empty()
                    && result.irreducible_blocks.is_empty()
            );
        } else {
            saw_success = true;
            assert_eq!(result, full);
        }
    }
    assert!(saw_exhaustion && saw_success);
}

#[test]
fn loop_irreducible_fallback_should_disable_loop_transforms_and_report_its_reason() {
    let module = loop_graph(&[&[1, 2], &[3, 4], &[3], &[1, 2], &[]]);
    let result = run_kir_pass_pipeline(module, KirOptimizationLevel::O3, None);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert!(result.artifact.is_some());
    assert!(
        result.analysis_fallbacks.iter().any(|fallback| {
            fallback.pass == "natural-loop-analysis"
                && fallback.reason == "irreducible-control-flow"
        }),
        "{:?}",
        result.analysis_fallbacks
    );
    assert_eq!(result.stats.hoisted_instructions, 0);
    assert_eq!(result.stats.induction_simplifications, 0);
    assert!(
        result
            .records
            .iter()
            .filter(|record| matches!(record.name.as_str(), "licm" | "induction-simplify"))
            .all(|record| !record.changed && record.verified)
    );
}

#[test]
fn loop_irreducible_budget_should_never_publish_a_partial_component() {
    let module = loop_graph(&[&[1], &[2, 3], &[4], &[4], &[2, 5], &[1, 6], &[]]);
    let function = &module.functions[0];
    let full = analyze_natural_loops(function);
    assert!(!full.irreducible_blocks.is_empty());
    let maximum = calckernel::ScalarAnalysisBudget::for_function(
        function,
        calckernel::ScalarAnalysisConfig::default(),
    )
    .max_steps();
    let mut successes = 0;
    for limit in 0..=maximum {
        let result = calckernel::analyze_natural_loops_with_config(
            function,
            calckernel::ScalarAnalysisConfig::with_max_steps(limit),
        );
        if result.budget_exhausted {
            assert!(
                result.irreducible_blocks.is_empty()
                    && result.loops.is_empty()
                    && result.inductions.is_empty()
            );
        } else {
            assert_eq!(result, full);
            successes += 1;
        }
    }
    assert!(successes > 0);
}

#[test]
fn loop_induction_simplify_should_remove_a_redundant_recurrence() {
    let source = "export fn count(n: u32) -> u32 { let i: u32 = 0; let j: u32 = 0; while i < n { i = i + 1; j = j + 1; } return j; }";
    for (level, expected_adds) in [(KirOptimizationLevel::O2, 2), (KirOptimizationLevel::O3, 1)] {
        let (kir, contracts) =
            build_with_modes(source, KirOverflowMode::Unchecked, KirBoundsMode::Unchecked);
        let result = run_kir_pass_pipeline(kir, level, contracts.as_ref());
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let artifact = result.artifact.expect("verified artifact");
        let additions = artifact.functions[0]
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .filter(|instruction| {
                matches!(
                    instruction.kind,
                    KirInstructionKind::Binary {
                        op: calckernel::MirBinaryOp::Add,
                        ..
                    }
                )
            })
            .count();
        assert_eq!(
            additions,
            expected_adds,
            "{level:?}: {}",
            print_kir_module(&artifact)
        );
        if level == KirOptimizationLevel::O3 {
            assert!(
                result
                    .records
                    .iter()
                    .any(|record| record.name == "induction-simplify" && record.changed)
            );
        }
    }
}

#[test]
fn loop_induction_simplify_should_cover_widths_directions_and_multiple_latches() {
    for integer in ["i32", "u32", "i64", "u64"] {
        for (comparison, body) in [
            ("i < n", "i = i + 2; j = j + 2;"),
            ("i > n", "i = i - 2; j = j - 2;"),
            (
                "i < n",
                "if choose { i = i + 1; j = j + 1; continue; } i = i + 2; j = j + 2;",
            ),
        ] {
            let source = format!(
                "export fn count(start: {integer}, n: {integer}, choose: bool) -> {integer} {{ let i: {integer} = start; let j: {integer} = start; while {comparison} {{ {body} }} return j; }}"
            );
            for overflow in [KirOverflowMode::Unchecked, KirOverflowMode::Checked] {
                let (kir, contracts) = build(&source, overflow);
                let result =
                    run_kir_pass_pipeline(kir, KirOptimizationLevel::O3, contracts.as_ref());
                assert!(
                    result.errors.is_empty(),
                    "{source}/{overflow:?}: {:?}",
                    result.errors
                );
                assert!(
                    result.stats.induction_simplifications > 0,
                    "{source}/{overflow:?}: {}",
                    print_kir_module(result.artifact.as_ref().expect("artifact"))
                );
            }
        }
    }
}

#[test]
fn loop_induction_simplify_should_reject_different_entries_and_any_unmatched_latch() {
    for source in [
        "export fn count(n: u32) -> u32 { let i: u32 = 0; let j: u32 = 1; while i < n { i = i + 1; j = j + 1; } return j; }",
        "export fn count(n: u32) -> u32 { let i: u32 = 0; let j: u32 = 0; while i < n { i = i + 1; j = j + 2; } return j; }",
        "export fn count(n: u32, choose: bool) -> u32 { let i: u32 = 0; let j: u32 = 0; while i < n { if choose { i = i + 1; continue; } i = i + 1; j = j + 1; } return j; }",
    ] {
        for overflow in [KirOverflowMode::Unchecked, KirOverflowMode::Checked] {
            let (kir, contracts) = build(source, overflow);
            let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O3, contracts.as_ref());
            assert!(result.errors.is_empty(), "{:?}", result.errors);
            assert_eq!(result.stats.induction_simplifications, 0, "{source}");
        }
    }
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
        export fn kernel(seed: i32) -> i32 {
          let i: i32 = 0;
          let total: i32 = seed;
          while i < 3 {
            print_i32(i + 1);
            total = total + seed;
            i = i + 1;
          }
          return total;
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
        .find(|function| function.name == "kernel")
        .expect("kernel");
    let loop_info = &analyze_natural_loops(function).loops[0];
    let ordered = function
        .blocks
        .iter()
        .filter(|block| loop_info.blocks.binary_search(&block.id).is_ok())
        .flat_map(|block| &block.instructions)
        .filter(|instruction| instruction.effect.is_some())
        .map(|instruction| &instruction.kind)
        .collect::<Vec<_>>();

    let print = ordered
        .iter()
        .position(|kind| matches!(kind, KirInstructionKind::RuntimeCall { .. }))
        .expect("runtime print remains ordered");
    let failure = ordered
        .iter()
        .position(|kind| matches!(kind, KirInstructionKind::Guard { .. }))
        .expect("unknown checked addition remains ordered");
    assert!(print < failure, "{ordered:?}");
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
fn signed_unit_induction_increment_should_be_proven_safe_at_o3() {
    let (kir, contracts) = build(
        r#"
        export fn fill(out: ptr<i64>, len: i32) -> i32 {
          let i: i32 = 0;
          while i < len {
            out[i] = 0;
            i = i + 1;
          }
          return 0;
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
        .find(|function| function.name == "fill")
        .expect("fill");
    let overflow_guards = function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .filter(|instruction| {
            matches!(
                instruction.kind,
                KirInstructionKind::Guard {
                    failure: KirFailureKind::Overflow,
                    ..
                }
            )
        })
        .count();

    assert_eq!(
        overflow_guards,
        0,
        "{:?}\n{:?}\n{}",
        result.explanations,
        analyze_natural_loops(function),
        print_kir_module(result.artifact.as_ref().expect("artifact"))
    );
}

#[test]
fn guard_loop_strict_bound_should_prove_only_the_current_integer_value() {
    for integer in ["i32", "u32", "i64", "u64"] {
        for (condition, body, expected_guards) in [
            ("i < bound", "i = i + 1;", 0),
            ("bound > i", "i = 1 + i;", 0),
            ("i <= bound", "i = i + 1;", 1),
            ("i < bound", "i = i + 2;", 1),
            ("i < bound", "i = bound; i = i + 1;", 1),
            ("i < bound", "if choose { i = other; } i = i + 1;", 1),
        ] {
            let source = format!(
                "export fn count(start: {integer}, bound: {integer}, other: {integer}, choose: bool) -> {integer} {{ let i: {integer} = start; while {condition} {{ {body} }} return i; }}"
            );
            for level in [KirOptimizationLevel::O2, KirOptimizationLevel::O3] {
                let (kir, contracts) = build(&source, KirOverflowMode::Checked);
                let result = run_kir_pass_pipeline(kir, level, contracts.as_ref());
                assert!(
                    result.errors.is_empty(),
                    "{integer}/{level:?}: {:?}",
                    result.errors
                );
                let artifact = result.artifact.expect("verified artifact");
                let guards = artifact.functions[0]
                    .blocks
                    .iter()
                    .flat_map(|block| &block.instructions)
                    .filter(|instruction| {
                        matches!(
                            instruction.kind,
                            KirInstructionKind::Guard {
                                failure: KirFailureKind::Overflow,
                                ..
                            }
                        )
                    })
                    .count();
                assert_eq!(
                    guards,
                    expected_guards,
                    "{source}\n{level:?}\n{}",
                    print_kir_module(&artifact)
                );
            }
        }
    }
}

#[test]
fn guard_loop_slice_identity_should_follow_all_edges_not_matching_slot_names() {
    let source = "export unsafe fn sum(items: slice<i32>, other: slice<i32>, len: u32) -> i32 contract { requires len <= items.len; effects read(other); } { let i: u32 = 0; let total: i32 = 0; while i < len { total = total + other[i]; i = i + 1; } return total; }";
    for level in [KirOptimizationLevel::O2, KirOptimizationLevel::O3] {
        let (mut kir, contracts) = build(source, KirOverflowMode::Checked);
        for param in kir.functions[0]
            .blocks
            .iter_mut()
            .flat_map(|block| &mut block.params)
        {
            param.slot = if param.slot == "other" {
                "items".to_string()
            } else {
                format!("anonymous_{}", param.value.index())
            };
        }
        assert!(calckernel::validate_kir_module(&kir).errors.is_empty());
        let result = run_kir_pass_pipeline(kir, level, contracts.as_ref());
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let artifact = result.artifact.expect("verified artifact");
        assert!(
            artifact.functions[0]
                .blocks
                .iter()
                .flat_map(|block| &block.instructions)
                .any(|instruction| matches!(
                    instruction.kind,
                    KirInstructionKind::Guard {
                        failure: KirFailureKind::OutOfBounds,
                        ..
                    }
                )),
            "a contract for items cannot justify indexing other:\n{}",
            print_kir_module(&artifact)
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
fn loop_canonical_slice_length_bound_should_prove_itself_without_a_contract() {
    let (kir, contracts) = build(
        r#"
        export fn maximum(items: slice<i64>, seed: i64) -> i64 {
          let i: u32 = 0;
          let result: i64 = seed;
          while i < items.len {
            if items[i] > result { result = items[i]; }
            i = i + 1;
          }
          return result;
        }
        "#,
        KirOverflowMode::Checked,
    );
    let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O3, contracts.as_ref());
    let function = result.artifact.as_ref().expect("artifact").functions[0].clone();
    assert!(
        function
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .all(|instruction| !matches!(
                instruction.kind,
                KirInstructionKind::Guard {
                    failure: KirFailureKind::OutOfBounds,
                    ..
                }
            )),
        "{}",
        print_kir_module(result.artifact.as_ref().expect("artifact"))
    );
}

#[test]
fn generated_loop_fixed_seed_should_validate_identically_at_every_level() {
    let generated = fixed_seed_kernel_program();
    let expected_names = generated
        .cases
        .iter()
        .map(|case| case.function.as_str())
        .collect::<Vec<_>>();
    assert!(
        generated
            .cases
            .iter()
            .all(|case| case.len <= case.values.len() as u32),
        "generated contract calls must stay inside the declared domain"
    );

    for overflow in [KirOverflowMode::Unchecked, KirOverflowMode::Checked] {
        for bounds in [KirBoundsMode::Unchecked, KirBoundsMode::Checked] {
            for level in [
                KirOptimizationLevel::O0,
                KirOptimizationLevel::O1,
                KirOptimizationLevel::O2,
                KirOptimizationLevel::O3,
            ] {
                let (kir, contracts) = build_with_modes(&generated.source, overflow, bounds);
                let result = run_kir_pass_pipeline(kir, level, contracts.as_ref());
                assert!(
                    result.errors.is_empty(),
                    "{overflow:?}/{bounds:?}/{level:?}: {:?}",
                    result.errors
                );
                let artifact = result.artifact.expect("generated verified artifact");
                assert_eq!(
                    artifact
                        .functions
                        .iter()
                        .filter(|function| function.exported)
                        .map(|function| function.name.as_str())
                        .collect::<Vec<_>>(),
                    expected_names,
                    "{overflow:?}/{bounds:?}/{level:?}"
                );
            }
        }
    }
}
