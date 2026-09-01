use calckernel::{
    ContractFactSet, KirBoundsMode, KirBuildConfig, KirConsumer, KirFailureKind,
    KirInstructionKind, KirOptimizationLevel, KirOverflowMode, KirSanitizerMode, SourceFile,
    TotalVersionPredicate, ValueId, VersionPredicateConjunct, analyze_affine_loop_accesses,
    analyze_canonical_loops, analyze_loop_dependences, analyze_natural_loops, build_kir_module,
    canonicalize_kir_loops, check, import_contract_facts, lower_to_mir, print_kir_module,
    run_kir_pass_pipeline, validate_canonical_loop_analysis,
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
            "loop-simplify",
            "check-elimination",
            "specialization-frontier",
            "effect-aware-inline",
            "memory-ssa-refine",
            "gvn",
            "load-forwarding",
            "dead-store-elimination",
            "sccp-range-post-inline",
            "check-elimination-post-inline",
            "natural-loop-analysis",
            "loop-legality-analysis",
            "licm",
            "induction-simplify",
            "sccp-range-post-loop",
            "check-elimination-post-loop",
            "loop-vector-frontier",
            "loop-optimization-frontier",
            "residual-slp-frontier",
            "dead-code-elimination",
            "cleanup",
        ]
    );
    assert!(result.records.iter().all(|record| record.verified));
    assert!(result.audit.attempts().is_empty());
    for frontier in [
        "specialization-frontier",
        "loop-vector-frontier",
        "loop-optimization-frontier",
        "residual-slp-frontier",
    ] {
        let record = result
            .records
            .iter()
            .find(|record| record.name == frontier)
            .expect("O3 empty frontier record");
        assert!(!record.changed);
        assert!(record.verified);
    }
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

#[test]
fn loop_induction_facts_should_cover_all_widths_directions_and_wrap_neighbors() {
    for ty in ["i32", "u32", "i64", "u64"] {
        for (condition, update, step, safe) in [
            ("i < n", "i + 1", "1", true),
            ("n > i", "i + 1", "1", true),
            ("i > n", "i - 1", "-1", true),
            ("n < i", "i - 1", "-1", true),
            ("i < n", "i + 2", "2", false),
            ("i > n", "i - 2", "-2", false),
            ("i <= n", "i + 1", "1", false),
            ("i >= n", "i - 1", "-1", false),
        ] {
            for overflow in [KirOverflowMode::Unchecked, KirOverflowMode::Checked] {
                let source = format!(
                    "export fn count(n: {ty}) -> {ty} {{ let i: {ty} = 10; while {condition} {{ i = {update}; }} return i; }}"
                );
                let (module, _) = build(&source, overflow);
                let analysis = analyze_natural_loops(&module.functions[0]);
                assert_eq!(analysis.inductions.len(), 1, "{source}");
                let induction = &analysis.inductions[0];
                assert_eq!(induction.start.to_string(), "10");
                assert_eq!(induction.step.to_string(), step);
                assert_eq!(
                    induction.wrap_safe_for_strict_bound, safe,
                    "{ty}: {condition}, {update}, {overflow:?}"
                );
            }
        }
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
    let function = &result.artifact.as_ref().expect("artifact").functions[0];
    let analysis = analyze_natural_loops(function);
    let multiply = function
        .blocks
        .iter()
        .find(|block| {
            block.instructions.iter().any(|instruction| {
                matches!(
                    instruction.kind,
                    KirInstructionKind::Binary {
                        op: calckernel::MirBinaryOp::Mul,
                        ..
                    }
                )
            })
        })
        .expect("invariant multiply");
    assert!(
        analysis
            .loops
            .iter()
            .all(|info| !info.blocks.contains(&multiply.id)),
        "the invariant multiply, not only its constants, must move:\n{}",
        print_kir_module(result.artifact.as_ref().expect("artifact"))
    );
}

#[test]
fn canonical_loop_analysis_should_retain_natural_loop_statistics_for_pipeline_reuse() {
    let (kir, _) = build(
        "export fn count(n: u32) -> u32 { let i: u32 = 0; while i < n { i = i + 1; } return i; }",
        KirOverflowMode::Unchecked,
    );
    let function = &kir.functions[0];
    let natural = analyze_natural_loops(function);
    let canonical = analyze_canonical_loops(function);

    assert_eq!(canonical.natural_loop_count as usize, natural.loops.len());
    assert_eq!(canonical.induction_count as usize, natural.inductions.len());
}

#[test]
fn loop_licm_should_not_speculate_integer_division_or_remainder() {
    for ty in ["i32", "u32", "i64", "u64"] {
        for op in ["/", "%"] {
            for overflow in [KirOverflowMode::Unchecked, KirOverflowMode::Checked] {
                let source = format!(
                    "export fn maybe(a: {ty}, d: {ty}, n: u32) -> {ty} {{ let i: u32 = 0; let total: {ty} = 0; while i < n {{ total = total + a {op} d; i = i + 1; }} return total; }}"
                );
                let (module, contracts) = build(&source, overflow);
                let result =
                    run_kir_pass_pipeline(module, KirOptimizationLevel::O3, contracts.as_ref());
                assert!(result.errors.is_empty(), "{:?}", result.errors);
                let function = &result.artifact.as_ref().expect("artifact").functions[0];
                let analysis = analyze_natural_loops(function);
                let divisions = function
                    .blocks
                    .iter()
                    .filter(|block| {
                        block.instructions.iter().any(|instruction| {
                            matches!(
                                instruction.kind,
                                KirInstructionKind::Binary {
                                    op: calckernel::MirBinaryOp::Div | calckernel::MirBinaryOp::Mod,
                                    ..
                                }
                            )
                        })
                    })
                    .collect::<Vec<_>>();
                assert_eq!(divisions.len(), 1);
                assert!(
                    analysis
                        .loops
                        .iter()
                        .any(|info| info.blocks.contains(&divisions[0].id)),
                    "division must not execute on a zero-trip path: {ty} {op} {overflow:?}\n{}",
                    print_kir_module(result.artifact.as_ref().expect("artifact"))
                );
            }
        }
    }
}

#[test]
fn loop_licm_should_keep_alias_memory_recursive_calls_and_strict_float_in_the_loop() {
    let source = r#"
        fn recurse(n: u32) -> u32 { if n == 0 { return 0; } return recurse(n - 1); }
        export fn effectful(out: ptr<u32>, input: ptr<u32>, a: u32, n: u32) -> u32 {
          let i: u32 = 0; let total: u32 = 0;
          while i < n { print_u32(i); out[0] = a; total = total + recurse(input[0]); i = i + 1; }
          return total;
        }
        export fn floats(a: f64, b: f64, n: u32) -> f64 {
          let i: u32 = 0; let total: f64 = 0.0;
          while i < n { total = total + a * b; i = i + 1; }
          return total;
        }
    "#;
    for overflow in [KirOverflowMode::Unchecked, KirOverflowMode::Checked] {
        let (module, contracts) = build(source, overflow);
        let result = run_kir_pass_pipeline(module, KirOptimizationLevel::O3, contracts.as_ref());
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let mut kinds = std::collections::BTreeSet::new();
        for function in result
            .artifact
            .as_ref()
            .expect("artifact")
            .functions
            .iter()
            .filter(|function| function.name != "recurse")
        {
            let analysis = analyze_natural_loops(function);
            for block in &function.blocks {
                for instruction in &block.instructions {
                    let kind = match instruction.kind {
                        KirInstructionKind::Load { .. } => "load",
                        KirInstructionKind::Store { .. } => "store",
                        KirInstructionKind::Call { .. } => "call",
                        KirInstructionKind::RuntimeCall { .. } => "print",
                        KirInstructionKind::Binary {
                            semantics: calckernel::KirArithmeticSemantics::StrictFloat,
                            ..
                        } => "strict-float",
                        _ => continue,
                    };
                    kinds.insert(kind);
                    assert!(
                        analysis
                            .loops
                            .iter()
                            .any(|info| info.blocks.contains(&block.id)),
                        "{kind} escaped its loop"
                    );
                }
            }
        }
        assert_eq!(
            kinds,
            std::collections::BTreeSet::from(["load", "store", "call", "print", "strict-float"])
        );
    }
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

#[test]
fn loop_simplify_should_form_one_preheader_latch_dedicated_exits_and_lcssa() {
    let (mut module, _) = build_with_modes(
        r#"
        export fn scan(values: slice<u32>, n: u32) -> u32 {
          let i: u32 = 0;
          let total: u32 = 0;
          while i < n {
            if i == 2 { i = i + 1; continue; }
            if i == 7 { break; }
            total = total + values[i];
            i = i + 1;
          }
          return total;
        }
        "#,
        KirOverflowMode::Unchecked,
        KirBoundsMode::Unchecked,
    );

    let first = canonicalize_kir_loops(&mut module).expect("loop simplify");
    assert!(first.changed, "continue must require a single-latch bridge");
    assert!(first.fallbacks.is_empty(), "{:?}", first.fallbacks);
    assert!(calckernel::validate_kir_module(&module).errors.is_empty());
    let analysis = analyze_canonical_loops(&module.functions[0]);
    assert!(analysis.fallbacks.is_empty(), "{analysis:?}");
    assert_eq!(analysis.loops.len(), 1);
    let descriptor = &analysis.loops[0];
    assert!(descriptor.preheader.is_some());
    assert!(descriptor.latch.is_some());
    assert!(descriptor.dedicated_exits);
    assert!(descriptor.lcssa);
    assert!(descriptor.innermost);
    validate_canonical_loop_analysis(&module.functions[0], &analysis).expect("fresh descriptor");

    let stable = analysis.clone();
    let second = canonicalize_kir_loops(&mut module).expect("idempotent loop simplify");
    assert!(!second.changed);
    assert_eq!(analyze_canonical_loops(&module.functions[0]), stable);
}

#[test]
fn loop_descriptor_should_be_nested_deterministic_and_invalidated_by_cfg_mutation() {
    let (mut module, _) = build_with_modes(
        r#"
        export fn nested(n: u32) -> u32 {
          let i: u32 = 0;
          let j: u32 = 0;
          let total: u32 = 0;
          while i < n {
            j = 0;
            while j < n { total = total + j; j = j + 1; }
            i = i + 1;
          }
          return total;
        }
        "#,
        KirOverflowMode::Unchecked,
        KirBoundsMode::Unchecked,
    );
    canonicalize_kir_loops(&mut module).expect("loop simplify");
    let analysis = analyze_canonical_loops(&module.functions[0]);
    assert_eq!(analysis.loops.len(), 2);
    assert!(
        analysis
            .loops
            .iter()
            .any(|item| item.depth == 2 && item.innermost)
    );
    assert!(
        analysis
            .loops
            .iter()
            .any(|item| item.depth == 1 && !item.innermost)
    );
    validate_canonical_loop_analysis(&module.functions[0], &analysis).expect("fresh descriptor");

    let header = analysis.loops[0].header;
    let block = module.functions[0]
        .blocks
        .iter_mut()
        .find(|block| block.id == header)
        .expect("header");
    let calckernel::KirTerminator::Branch { then_edge, .. } = &mut block.terminator else {
        panic!("loop header branch")
    };
    then_edge.args.swap(0, 1);
    let error = validate_canonical_loop_analysis(&module.functions[0], &analysis)
        .expect_err("stale CFG descriptor");
    assert!(error.contains("stale"), "{error}");
}

#[test]
fn loop_descriptor_should_distinguish_zero_exact_runtime_and_noncountable_trips() {
    for (start, bound, expected) in [("0", "8", Some(8_u64)), ("8", "8", Some(0_u64))] {
        let source = format!(
            "export fn count() -> u32 {{ let i: u32 = {start}; while i < {bound} {{ i = i + 1; }} return i; }}"
        );
        let (mut module, _) = build_with_modes(
            &source,
            KirOverflowMode::Unchecked,
            KirBoundsMode::Unchecked,
        );
        canonicalize_kir_loops(&mut module).expect("loop simplify");
        let loops = analyze_canonical_loops(&module.functions[0]);
        match (&loops.loops[0].trip_count, expected) {
            (calckernel::LoopTripCount::Zero, Some(0)) => {}
            (calckernel::LoopTripCount::Exact { iterations }, Some(expected)) => {
                assert_eq!(*iterations, expected)
            }
            (actual, _) => panic!("unexpected trip classification: {actual:?}"),
        }
    }

    let (mut runtime, _) = build_with_modes(
        "export fn count(n: u32) -> u32 { let i: u32 = 0; while i < n { i = i + 1; } return i; }",
        KirOverflowMode::Unchecked,
        KirBoundsMode::Unchecked,
    );
    canonicalize_kir_loops(&mut runtime).expect("loop simplify");
    let loops = analyze_canonical_loops(&runtime.functions[0]);
    assert!(matches!(
        loops.loops[0].trip_count,
        calckernel::LoopTripCount::Runtime { .. }
    ));

    let (mut noncountable, _) = build_with_modes(
        "export fn count(n: u32) -> u32 { let i: u32 = 0; while i <= n { i = i + 1; } return i; }",
        KirOverflowMode::Unchecked,
        KirBoundsMode::Unchecked,
    );
    canonicalize_kir_loops(&mut noncountable).expect("loop simplify");
    let loops = analyze_canonical_loops(&noncountable.functions[0]);
    assert!(matches!(
        loops.loops[0].trip_count,
        calckernel::LoopTripCount::Unknown
    ));
    assert!(
        loops.fallbacks.iter().any(|fallback| {
            fallback.reason == calckernel::LoopFallbackReason::NonCountableTrip
        })
    );
}

#[test]
fn loop_descriptor_budget_exhaustion_should_return_one_stable_scalar_fallback() {
    let (module, _) = build_with_modes(
        "export fn count(n: u32) -> u32 { let i: u32 = 0; while i < n { i = i + 1; } return i; }",
        KirOverflowMode::Unchecked,
        KirBoundsMode::Unchecked,
    );
    let analysis = calckernel::analyze_canonical_loops_with_config(
        &module.functions[0],
        calckernel::ScalarAnalysisConfig::with_max_steps(0),
    );
    assert!(analysis.loops.is_empty());
    assert!(analysis.budget_exhausted);
    assert_eq!(
        analysis.fallbacks[0].reason.stable_name(),
        "fixed-loop-budget-exhausted"
    );
}

#[test]
fn affine_access_should_extract_unit_stride_bias_alignment_and_reject_nonunit_groups() {
    let (mut module, _) = build_with_modes(
        r#"
        export fn affine(input: slice<u32>, output: slice<u32>, n: u32) -> void {
          let i: u32 = 0;
          while i < n {
            output[i] = input[i + 1];
            output[i + i] = input[n - i];
            i = i + 1;
          }
        }
        "#,
        KirOverflowMode::Unchecked,
        KirBoundsMode::Unchecked,
    );
    canonicalize_kir_loops(&mut module).expect("loop simplify");
    let loops = analyze_canonical_loops(&module.functions[0]);
    let descriptor = loops
        .loops
        .iter()
        .find(|item| item.innermost)
        .expect("loop");
    let accesses = analyze_affine_loop_accesses(&module.functions[0], descriptor, None)
        .expect("affine accesses");

    assert!(
        accesses.accesses.iter().any(|access| {
            access.unit_stride && access.bias.to_string() == "1" && access.vector_group_eligible
        }),
        "{accesses:?}"
    );
    assert!(
        accesses.accesses.iter().any(|access| {
            access.coefficient.to_string() == "2" && !access.vector_group_eligible
        }),
        "{accesses:?}"
    );
    assert!(
        accesses.accesses.iter().any(|access| {
            access.coefficient.to_string() == "-1" && !access.vector_group_eligible
        }),
        "{accesses:?}"
    );
    assert!(
        accesses
            .accesses
            .iter()
            .all(|access| access.element_bytes == 4)
    );

    let (mut aligned, contracts) = build_with_modes(
        r#"
        export unsafe fn aligned(a: slice<u32>, n: u32) -> u32
        contract { requires aligned(a.data, 32); effects read(a); }
        {
          let i: u32 = 0;
          let total: u32 = 0;
          while i < n { total = total + a[i]; i = i + 1; }
          return total;
        }
        "#,
        KirOverflowMode::Unchecked,
        KirBoundsMode::Unchecked,
    );
    canonicalize_kir_loops(&mut aligned).expect("loop simplify");
    let loops = analyze_canonical_loops(&aligned.functions[0]);
    let accesses = analyze_affine_loop_accesses(
        &aligned.functions[0],
        &loops.loops[0],
        contracts.as_ref().map(ContractFactSet::facts),
    )
    .expect("aligned access");
    assert!(
        accesses
            .accesses
            .iter()
            .all(|access| access.base_alignment >= 32 && access.known_alignment >= 4),
        "{accesses:?}"
    );

    let (mut subslice, _) = build_with_modes(
        r#"
        export fn windowed(input: slice<u32>, n: u32) -> u32 {
          let window: slice<u32> = input[1..n];
          let i: u32 = 0;
          let total: u32 = 0;
          while i < window.len { total = total + window[i]; i = i + 1; }
          return total;
        }
        "#,
        KirOverflowMode::Unchecked,
        KirBoundsMode::Unchecked,
    );
    canonicalize_kir_loops(&mut subslice).expect("loop simplify");
    let loops = analyze_canonical_loops(&subslice.functions[0]);
    let accesses = analyze_affine_loop_accesses(&subslice.functions[0], &loops.loops[0], None)
        .expect("subslice access");
    assert!(
        accesses
            .accesses
            .iter()
            .any(|access| access.slice_interval.is_some()),
        "{accesses:?}"
    );
}

#[test]
fn dependence_should_classify_noalias_dependent_and_runtime_guarded_write_pairs() {
    let (mut module, contracts) = build_with_modes(
        r#"
        export unsafe fn classify(a: slice<u32>, b: slice<u32>, n: u32) -> void
        contract { requires noalias(a, b); effects read(a), write(b); }
        {
          let i: u32 = 0;
          while i < n { b[i] = a[i]; i = i + 1; }
        }
        "#,
        KirOverflowMode::Unchecked,
        KirBoundsMode::Unchecked,
    );
    canonicalize_kir_loops(&mut module).expect("loop simplify");
    let loops = analyze_canonical_loops(&module.functions[0]);
    let report = analyze_loop_dependences(
        &module.functions[0],
        &loops.loops[0],
        contracts.as_ref().map(ContractFactSet::facts),
    )
    .expect("dependence report");
    assert!(
        report
            .pairs
            .iter()
            .any(|pair| { pair.kind == calckernel::LoopDependenceKind::Independent })
    );

    let (mut unknown, _) = build_with_modes(
        r#"
        export fn classify(a: slice<u32>, b: slice<u32>, n: u32) -> void {
          let i: u32 = 0;
          while i < n { b[i] = a[i]; i = i + 1; }
        }
        "#,
        KirOverflowMode::Unchecked,
        KirBoundsMode::Unchecked,
    );
    canonicalize_kir_loops(&mut unknown).expect("loop simplify");
    let loops = analyze_canonical_loops(&unknown.functions[0]);
    let report = analyze_loop_dependences(&unknown.functions[0], &loops.loops[0], None)
        .expect("unknown dependence report");
    assert!(report.pairs.iter().any(|pair| {
        pair.kind == calckernel::LoopDependenceKind::RuntimeGuarded && pair.predicate.is_some()
    }));

    let (mut same, _) = build_with_modes(
        r#"
        export fn shift(a: slice<u32>, n: u32) -> void {
          let i: u32 = 0;
          while i < n { a[i + 1] = a[i]; i = i + 1; }
        }
        "#,
        KirOverflowMode::Unchecked,
        KirBoundsMode::Unchecked,
    );
    canonicalize_kir_loops(&mut same).expect("loop simplify");
    let loops = analyze_canonical_loops(&same.functions[0]);
    let report = analyze_loop_dependences(&same.functions[0], &loops.loops[0], None)
        .expect("dependent report");
    assert!(
        report
            .pairs
            .iter()
            .any(|pair| { pair.kind == calckernel::LoopDependenceKind::Dependent })
    );

    let (mut reduction, _) = build_with_modes(
        r#"
        export fn sum(a: slice<u32>, n: u32) -> u32 {
          let i: u32 = 0;
          let total: u32 = 0;
          while i < n { total = total + a[i]; i = i + 1; }
          return total;
        }
        "#,
        KirOverflowMode::Unchecked,
        KirBoundsMode::Unchecked,
    );
    canonicalize_kir_loops(&mut reduction).expect("loop simplify");
    let loops = analyze_canonical_loops(&reduction.functions[0]);
    let report = analyze_loop_dependences(&reduction.functions[0], &loops.loops[0], None)
        .expect("reduction report");
    assert_eq!(report.reductions.len(), 1, "{report:?}");
    assert_eq!(report.reductions[0].operation, calckernel::MirBinaryOp::Add);

    let (mut reads, _) = build_with_modes(
        r#"
        export fn reads(a: slice<u32>, b: slice<u32>, n: u32) -> u32 {
          let i: u32 = 0;
          let total: u32 = 0;
          while i < n { total = total + a[i] + b[i]; i = i + 1; }
          return total;
        }
        "#,
        KirOverflowMode::Unchecked,
        KirBoundsMode::Unchecked,
    );
    canonicalize_kir_loops(&mut reads).expect("loop simplify");
    let loops = analyze_canonical_loops(&reads.functions[0]);
    let report = analyze_loop_dependences(&reads.functions[0], &loops.loops[0], None)
        .expect("read/read report");
    assert!(
        report
            .pairs
            .iter()
            .any(|pair| pair.kind == calckernel::LoopDependenceKind::ReadRead)
    );

    let (mut raw, _) = build_with_modes(
        r#"
        export fn raw(a: ptr<u32>, b: ptr<u32>, n: u32) -> void {
          let i: u32 = 0;
          while i < n { b[i] = a[i]; i = i + 1; }
        }
        "#,
        KirOverflowMode::Unchecked,
        KirBoundsMode::Unchecked,
    );
    canonicalize_kir_loops(&mut raw).expect("loop simplify");
    let loops = analyze_canonical_loops(&raw.functions[0]);
    let report = analyze_loop_dependences(&raw.functions[0], &loops.loops[0], None)
        .expect("raw pointer report");
    assert!(report.pairs.iter().any(|pair| {
        pair.kind == calckernel::LoopDependenceKind::Unknown && pair.predicate.is_none()
    }));
}

#[test]
fn dependence_ordered_effect_should_preserve_scalar_first_error_and_print_order() {
    for source in [
        "export fn noisy(n: u32) -> void { let i: u32 = 0; while i < n { print_u32(i); i = i + 1; } }",
        "export fn checked(a: slice<u32>, n: u32) -> u32 { let i: u32 = 0; let total: u32 = 0; while i < n { total = total + a[i]; i = i + 1; } return total; }",
    ] {
        let (mut module, _) = build(source, KirOverflowMode::Checked);
        canonicalize_kir_loops(&mut module).expect("loop simplify");
        let loops = analyze_canonical_loops(&module.functions[0]);
        let descriptor = loops
            .loops
            .iter()
            .find(|item| item.innermost)
            .expect("loop");
        let legality = calckernel::analyze_loop_legality(&module.functions[0], descriptor, None)
            .expect("legality");
        assert!(!legality.eligible, "{legality:?}");
        assert!(
            legality
                .fallback_reasons
                .iter()
                .any(|reason| { *reason == calckernel::LoopFallbackReason::OrderedEffect }),
            "{legality:?}"
        );
    }
}

#[test]
fn version_predicate_should_be_total_bounded_and_false_on_target_width_overflow() {
    let left = ValueId::from_index(0);
    let right = ValueId::from_index(1);
    let count = ValueId::from_index(2);
    let predicate = TotalVersionPredicate {
        address_bits: 32,
        conjuncts: vec![VersionPredicateConjunct::AddressIntervalsDisjoint {
            left,
            left_count: count,
            left_element_bytes: 4,
            right,
            right_count: count,
            right_element_bytes: 4,
        }],
    };
    predicate.validate().expect("closed predicate");

    let values = std::collections::BTreeMap::from([(left, 0_u64), (right, 64), (count, 8)]);
    assert!(predicate.evaluate(&values));
    let overlapping = std::collections::BTreeMap::from([(left, 0_u64), (right, 16), (count, 8)]);
    assert!(!predicate.evaluate(&overlapping));
    let overflow =
        std::collections::BTreeMap::from([(left, u64::from(u32::MAX) - 3), (right, 0), (count, 2)]);
    assert!(!predicate.evaluate(&overflow));
    let multiply_overflow = TotalVersionPredicate {
        address_bits: 64,
        conjuncts: vec![VersionPredicateConjunct::AddressIntervalsDisjoint {
            left,
            left_count: count,
            left_element_bytes: 2,
            right,
            right_count: count,
            right_element_bytes: 2,
        }],
    };
    let multiply_overflow_values =
        std::collections::BTreeMap::from([(left, 0), (right, 1), (count, u64::MAX)]);
    assert!(!multiply_overflow.evaluate(&multiply_overflow_values));
    let empty = std::collections::BTreeMap::from([
        (left, u64::from(u32::MAX)),
        (right, u64::from(u32::MAX)),
        (count, 0),
    ]);
    assert!(
        predicate.evaluate(&empty),
        "zero footprint forms no end address"
    );

    let scalar_predicates = TotalVersionPredicate {
        address_bits: 64,
        conjuncts: vec![
            VersionPredicateConjunct::TripThreshold {
                trip_count: count,
                minimum: 8,
            },
            VersionPredicateConjunct::Divisible {
                value: count,
                divisor: 4,
            },
            VersionPredicateConjunct::PowerOfTwoAlignment {
                address: left,
                alignment: 32,
            },
        ],
    };
    assert!(
        scalar_predicates.evaluate(&std::collections::BTreeMap::from([(left, 64), (count, 8),]))
    );
    assert!(
        !scalar_predicates.evaluate(&std::collections::BTreeMap::from([(left, 68), (count, 8),]))
    );
    assert!(!scalar_predicates.evaluate(&std::collections::BTreeMap::new()));

    let mut too_many = predicate.clone();
    too_many.conjuncts = (0..5)
        .map(|_| VersionPredicateConjunct::TripThreshold {
            trip_count: count,
            minimum: 1,
        })
        .collect();
    assert!(too_many.validate().is_err());
}
