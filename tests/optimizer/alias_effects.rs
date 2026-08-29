use std::collections::BTreeSet;

use calckernel::{
    EffectAccess, EffectCall, EffectFunction, EffectGraph, EffectSolveConfig, EffectTarget,
    KirBoundsMode, KirBuildConfig, KirConsumer, KirInstructionKind, KirOverflowMode,
    KirSanitizerMode, MemoryEffect, SourceFile, build_kir_module, check, import_contract_facts,
    lower_to_mir, refine_memory_ssa, refine_memory_ssa_with_effects, solve_effect_graph,
    solve_kir_effects,
};

fn build(source_text: &str) -> (calckernel::CheckedProgram, calckernel::KirModule) {
    let checked = check(&SourceFile::new("alias.ck", source_text));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR lowering");
    let kir = build_kir_module(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::Inspection,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
    )
    .expect("KIR construction");
    (checked.checked_program, kir)
}

#[test]
fn alias_memory_ssa_should_partition_two_pairwise_noalias_slice_roots() {
    let (checked, mut kir) = build(
        r#"
        export unsafe fn copy(a: slice<i32>, b: slice<i32>) -> void
        contract { requires noalias(a, b); effects read(a), write(b); }
        { b[0] = a[0]; }
        "#,
    );
    let contracts = import_contract_facts(&kir, &checked, 0).expect("contracts");
    let report = refine_memory_ssa(&mut kir, Some(contracts.facts())).expect("Memory SSA");
    let function = &kir.functions[0];
    let accesses = function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .filter_map(|instruction| instruction.memory.as_ref())
        .collect::<Vec<_>>();

    assert_eq!(report.functions[0].partition_count, 2);
    assert_eq!(function.initial_memory.len(), 2);
    assert_ne!(accesses[0].region, accesses[1].region);
    assert_eq!(calckernel::validate_kir_module(&kir).errors, []);
}

#[test]
fn alias_memory_ssa_should_merge_pairwise_fact_when_a_third_root_may_alias_both() {
    let (checked, mut kir) = build(
        r#"
        export unsafe fn update(a: slice<i32>, b: slice<i32>, c: slice<i32>) -> void
        contract { requires noalias(a, b); effects read(a), read(b), write(c); }
        { c[0] = a[0] + b[0]; }
        "#,
    );
    let contracts = import_contract_facts(&kir, &checked, 0).expect("contracts");
    let report = refine_memory_ssa(&mut kir, Some(contracts.facts())).expect("Memory SSA");

    assert_eq!(report.functions[0].partition_count, 1);
    assert_eq!(kir.functions[0].initial_memory.len(), 1);
}

#[test]
fn alias_memory_ssa_should_use_conservative_partition_for_unknown_calls() {
    let (checked, mut kir) = build(
        r#"
        unsafe fn touch(a: slice<i32>) -> void
        contract { requires a.len > 0; }
        { a[0] = 1; }

        export unsafe fn caller(a: slice<i32>, b: slice<i32>) -> void
        contract { requires noalias(a, b); }
        { unsafe { touch(a); } b[0] = 2; }
        "#,
    );
    let contracts = import_contract_facts(&kir, &checked, 0).expect("contracts");
    let report = refine_memory_ssa(&mut kir, Some(contracts.facts())).expect("Memory SSA");
    let caller_index = kir
        .functions
        .iter()
        .position(|f| f.name == "caller")
        .expect("caller");
    let caller = &kir.functions[caller_index];
    let call = caller
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .find(|instruction| matches!(instruction.kind, KirInstructionKind::Call { .. }))
        .expect("call");

    assert!(report.functions[caller_index].collapsed_for_call);
    assert_eq!(report.functions[caller_index].partition_count, 1);
    assert_eq!(
        call.memory.as_ref().expect("call memory").region,
        caller.regions[0].id
    );
}

#[test]
fn effect_solver_should_map_direct_transitive_and_recursive_parameter_effects() {
    let graph = EffectGraph {
        functions: vec![
            EffectFunction {
                name: "read".to_string(),
                parameter_count: 1,
                direct: calckernel::EffectSummary::from_access(EffectAccess {
                    target: EffectTarget::Parameter(0),
                    effect: MemoryEffect::Read,
                }),
                calls: Vec::new(),
            },
            EffectFunction {
                name: "forward".to_string(),
                parameter_count: 2,
                direct: calckernel::EffectSummary::empty(),
                calls: vec![EffectCall {
                    callee: "read".to_string(),
                    arguments: vec![Some(EffectTarget::Parameter(1))],
                    is_unsafe: false,
                }],
            },
            EffectFunction {
                name: "recursive".to_string(),
                parameter_count: 1,
                direct: calckernel::EffectSummary::from_access(EffectAccess {
                    target: EffectTarget::Parameter(0),
                    effect: MemoryEffect::Write,
                }),
                calls: vec![EffectCall {
                    callee: "recursive".to_string(),
                    arguments: vec![Some(EffectTarget::Parameter(0))],
                    is_unsafe: false,
                }],
            },
        ],
    };
    let result = solve_effect_graph(&graph, EffectSolveConfig::default());

    assert!(!result.exhausted);
    assert_eq!(result.sccs.len(), 3);
    assert_eq!(
        result.summaries["forward"].effect(EffectTarget::Parameter(1)),
        MemoryEffect::Read
    );
    assert_eq!(
        result.summaries["forward"].effect(EffectTarget::Parameter(0)),
        MemoryEffect::None
    );
    assert_eq!(
        result.summaries["recursive"].effect(EffectTarget::Parameter(0)),
        MemoryEffect::Write
    );
}

#[test]
fn effect_solver_should_fallback_to_full_conservative_summary_on_unknown_or_budget() {
    let unknown_graph = EffectGraph {
        functions: vec![EffectFunction {
            name: "caller".to_string(),
            parameter_count: 1,
            direct: calckernel::EffectSummary::empty(),
            calls: vec![EffectCall {
                callee: "external".to_string(),
                arguments: vec![Some(EffectTarget::Parameter(0))],
                is_unsafe: true,
            }],
        }],
    };
    let unknown = solve_effect_graph(&unknown_graph, EffectSolveConfig::default());
    let summary = &unknown.summaries["caller"];
    assert_eq!(summary.effect(EffectTarget::All), MemoryEffect::ReadWrite);
    assert!(summary.may_fail && summary.runtime_effect && summary.unsafe_calls);

    let recursive_graph = EffectGraph {
        functions: vec![EffectFunction {
            name: "loop".to_string(),
            parameter_count: 1,
            direct: calckernel::EffectSummary::empty(),
            calls: vec![EffectCall {
                callee: "loop".to_string(),
                arguments: vec![Some(EffectTarget::Parameter(0))],
                is_unsafe: false,
            }],
        }],
    };
    let constrained = solve_effect_graph(&recursive_graph, EffectSolveConfig::with_max_steps(0));
    assert!(constrained.exhausted);
    assert_eq!(
        constrained.summaries["loop"].effect(EffectTarget::All),
        MemoryEffect::ReadWrite
    );
    assert!(constrained.summaries["loop"].may_fail);
}

#[test]
fn runtime_print_effect_should_remain_independent_from_memory_ceiling() {
    let mut direct = calckernel::EffectSummary::empty();
    direct.runtime_effect = true;
    direct.may_fail = true;
    let graph = EffectGraph {
        functions: vec![EffectFunction {
            name: "observable".to_string(),
            parameter_count: 1,
            direct,
            calls: Vec::new(),
        }],
    };
    let result = solve_effect_graph(&graph, EffectSolveConfig::default());
    let summary = &result.summaries["observable"];

    assert_eq!(
        summary.effect(EffectTarget::Parameter(0)),
        MemoryEffect::None
    );
    assert!(summary.runtime_effect);
    assert!(summary.may_fail);
}

#[test]
fn effect_kir_adapter_should_use_the_same_lattice_mapping_and_scc_solver_as_source() {
    let checked = check(&SourceFile::new(
        "effect.ck",
        r#"
        fn write_first(items: slice<i32>) -> void { items[0] = 1; }
        export unsafe fn kernel(items: slice<i32>, n: i32) -> void
        contract { requires items.len > 0; effects write(items); }
        { write_first(items); print_i32(n + 1); }
        "#,
    ));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR");
    let kir = build_kir_module(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::Inspection,
            overflow_mode: KirOverflowMode::Checked,
            bounds_mode: KirBoundsMode::Checked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
    )
    .expect("KIR");
    let unsafe_functions = checked
        .checked_program
        .functions
        .iter()
        .filter(|function| function.is_unsafe)
        .map(|function| function.name.clone())
        .collect::<BTreeSet<_>>();
    let effects = solve_kir_effects(&kir, &unsafe_functions, EffectSolveConfig::default());
    let source = &checked.checked_program.effect_summaries["kernel"];
    let from_kir = &effects.summaries["kernel"];

    assert_eq!(
        from_kir.effect(EffectTarget::Parameter(0)),
        source.effect(EffectTarget::Parameter(0))
    );
    assert_eq!(from_kir.runtime_effect, source.runtime_effect);
    assert_eq!(from_kir.may_fail, source.may_fail);
    assert_eq!(from_kir.unsafe_calls, source.unsafe_calls);
}

#[test]
fn effect_memory_ssa_should_use_parameter_mapped_callee_summary_without_clobbering_other_partition()
{
    let (checked, mut kir) = build(
        r#"
        fn write_first(items: slice<i32>) -> void { items[0] = 1; }
        export unsafe fn caller(a: slice<i32>, b: slice<i32>) -> void
        contract { requires noalias(a, b); effects write(a), write(b); }
        { write_first(a); b[0] = 2; }
        "#,
    );
    let unsafe_functions = checked
        .functions
        .iter()
        .filter(|function| function.is_unsafe)
        .map(|function| function.name.clone())
        .collect::<BTreeSet<_>>();
    let effects = solve_kir_effects(&kir, &unsafe_functions, EffectSolveConfig::default());
    let contracts = import_contract_facts(&kir, &checked, 0).expect("contracts");
    let report =
        refine_memory_ssa_with_effects(&mut kir, Some(contracts.facts()), &effects.summaries)
            .expect("effect-aware Memory SSA");
    let caller_index = kir
        .functions
        .iter()
        .position(|f| f.name == "caller")
        .expect("caller");
    let caller = &kir.functions[caller_index];
    let accesses = caller
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .filter_map(|instruction| instruction.memory.as_ref())
        .map(|memory| memory.region)
        .collect::<Vec<_>>();

    assert!(!report.functions[caller_index].collapsed_for_call);
    assert_eq!(report.functions[caller_index].partition_count, 2);
    assert_ne!(accesses[0], accesses[1]);
    assert_eq!(calckernel::validate_kir_module(&kir).errors, []);
}
