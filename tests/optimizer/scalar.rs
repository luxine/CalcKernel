use std::collections::BTreeMap;

use calckernel::{
    AffineForm, BlockId, BoolLattice, ContractInstanceSource, FactScope, FactUseSite, IntegerType,
    KirArithmeticSemantics, KirBoundsMode, KirBuildConfig, KirConsumer, KirOverflowMode,
    KirSanitizerMode, MirBinaryOp, MirCompareOp, ScalarAnalysisConfig, ScalarCongruence,
    ScalarFailure, ScalarInterval, ScalarValue, SourceFile, ValueId, analyze_scalar_function,
    build_kir_module, check, clone_contract_instance_for_inline, contract_fact_dominates_at,
    import_contract_facts, lower_to_mir, materialize_scalar_facts, narrow_scalar, print_fact_arena,
    refine_scalar_comparison, scalar_binary, scalar_compare, widen_scalar,
};
use num_bigint::BigInt;

fn checked_kir(source_text: &str) -> (calckernel::CheckedProgram, calckernel::KirModule) {
    let checked = check(&SourceFile::new("scalar.ck", source_text));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR lowering");
    let kir = build_kir_module(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::Inspection,
            overflow_mode: KirOverflowMode::Checked,
            bounds_mode: KirBoundsMode::Checked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
    )
    .expect("KIR construction");
    (checked.checked_program, kir)
}

#[test]
fn contract_fact_import_should_substitute_entry_and_unsafe_call_instances() {
    let (checked, kir) = checked_kir(
        r#"
        unsafe fn consume(items: slice<i32>, n: u32) -> u32
        contract {
          requires n + 1 <= items.len;
          requires multiple_of(n, 4);
          effects read(items);
        }
        { return n; }

        export unsafe fn caller(items: slice<i32>, n: u32) -> u32
        contract { requires n + 1 <= items.len; effects read(items); }
        { unsafe { return consume(items, n); } }
        "#,
    );
    let imported = import_contract_facts(&kir, &checked, 0).expect("contract facts");
    let consume = kir
        .functions
        .iter()
        .find(|function| function.name == "consume")
        .expect("consume KIR");
    let caller = kir
        .functions
        .iter()
        .find(|function| function.name == "caller")
        .expect("caller KIR");
    let consume_instances = imported
        .instances()
        .iter()
        .filter(|instance| instance.callee == consume.id)
        .collect::<Vec<_>>();

    assert_eq!(consume_instances.len(), 2);
    assert!(matches!(
        consume_instances[0].source,
        ContractInstanceSource::FunctionEntry
    ));
    let ContractInstanceSource::Call {
        caller: call_owner,
        block,
        instruction,
    } = consume_instances[1].source
    else {
        panic!("expected call instance");
    };
    assert_eq!(call_owner, caller.id);
    assert_eq!(consume_instances[1].bindings.len(), 2);
    assert_ne!(
        consume_instances[0].bindings[1].value,
        consume_instances[1].bindings[1].value
    );
    assert!(print_fact_arena(imported.facts()).contains("contract-compare("));

    let call_fact = consume_instances[1].facts[0];
    assert!(matches!(
        imported.facts().get(call_fact).expect("call fact").scope,
        FactScope::CalleeInstance { .. }
    ));
    assert!(!contract_fact_dominates_at(
        &imported,
        call_fact,
        FactUseSite {
            function: caller.id,
            block,
            instruction: Some(instruction),
            contract_instance: None,
        }
    ));
    assert!(contract_fact_dominates_at(
        &imported,
        call_fact,
        FactUseSite {
            function: consume.id,
            block: consume.blocks[0].id,
            instruction: None,
            contract_instance: Some(consume_instances[1].id),
        }
    ));
}

#[test]
fn contract_fact_inline_clone_scope_should_not_escape_to_unrelated_blocks() {
    let (checked, kir) = checked_kir(
        r#"
        unsafe fn callee(n: u32) -> u32
        contract { requires n < 8; }
        { return n; }

        export unsafe fn caller(n: u32) -> u32
        contract { requires n < 8; }
        { unsafe { return callee(n); } }
        "#,
    );
    let mut imported = import_contract_facts(&kir, &checked, 4).expect("contract facts");
    let caller = kir
        .functions
        .iter()
        .find(|f| f.name == "caller")
        .expect("caller");
    let callee = kir
        .functions
        .iter()
        .find(|f| f.name == "callee")
        .expect("callee");
    let source = imported
        .instances()
        .iter()
        .find(|instance| {
            instance.callee == callee.id
                && matches!(instance.source, ContractInstanceSource::Call { .. })
        })
        .expect("call instance")
        .id;
    let clone_block = caller.blocks[0].id;
    let unrelated = BlockId::from_index(u32::MAX - 1);
    let value_map = BTreeMap::<ValueId, ValueId>::new();
    let clone = clone_contract_instance_for_inline(
        &mut imported,
        source,
        caller.id,
        9,
        vec![clone_block],
        &value_map,
    )
    .expect("inline contract clone");
    let cloned = imported
        .instances()
        .iter()
        .find(|instance| instance.id == clone)
        .expect("cloned instance");
    let fact = cloned.facts[0];

    assert!(contract_fact_dominates_at(
        &imported,
        fact,
        FactUseSite {
            function: caller.id,
            block: clone_block,
            instruction: None,
            contract_instance: Some(clone),
        }
    ));
    assert!(!contract_fact_dominates_at(
        &imported,
        fact,
        FactUseSite {
            function: caller.id,
            block: unrelated,
            instruction: None,
            contract_instance: Some(clone),
        }
    ));
}

#[test]
fn contract_fact_recursive_call_edge_should_receive_a_fresh_instance() {
    let (checked, kir) = checked_kir(
        r#"
        export unsafe fn recurse(n: u32) -> u32
        contract { requires n < 8; }
        {
          if n > 0 { unsafe { return recurse(n - 1); } }
          return n;
        }
        "#,
    );
    let imported = import_contract_facts(&kir, &checked, 0).expect("contract facts");
    let recurse = kir
        .functions
        .iter()
        .find(|f| f.name == "recurse")
        .expect("recurse");
    let instances = imported
        .instances()
        .iter()
        .filter(|instance| instance.callee == recurse.id)
        .collect::<Vec<_>>();

    assert_eq!(instances.len(), 2);
    assert_ne!(instances[0].id, instances[1].id);
    assert_ne!(instances[0].facts, instances[1].facts);
    assert!(matches!(
        instances[1].source,
        ContractInstanceSource::Call { .. }
    ));
}

fn int(value: i128) -> BigInt {
    BigInt::from(value)
}

#[test]
fn scalar_domain_property_cases_should_cover_intervals_congruence_known_bits_and_affine() {
    let cases = [
        (IntegerType::I32, i32::MIN as i128, i32::MAX as i128),
        (IntegerType::I64, i64::MIN as i128, i64::MAX as i128),
        (IntegerType::U32, 0, u32::MAX as i128),
        (IntegerType::U64, 0, u64::MAX as i128),
    ];
    for (type_node, minimum, maximum) in cases {
        let top = ScalarValue::unknown(type_node);
        assert_eq!(top.interval().lower(), &int(minimum));
        assert_eq!(top.interval().upper(), &int(maximum));
        assert!(top.is_unknown());

        for value in [minimum, 0, maximum] {
            let exact = ScalarValue::constant(type_node, int(value)).expect("in-range constant");
            assert_eq!(exact.exact_value(), Some(&int(value)));
            assert!(exact.congruence().contains(&int(value)));
            assert!(exact.known_bits().matches(&int(value), type_node));
        }
    }

    let even = ScalarCongruence::new(int(2), int(0)).expect("even congruence");
    let odd = ScalarCongruence::new(int(2), int(1)).expect("odd congruence");
    assert!(even.add(&odd).contains(&int(7)));
    assert!(!even.add(&odd).contains(&int(8)));

    let x = AffineForm::variable(ValueId::from_index(4));
    let expression = x.scale(&int(3)).add_constant(int(-2));
    assert_eq!(expression.coefficient(ValueId::from_index(4)), int(3));
    assert_eq!(expression.constant(), &int(-2));
}

#[test]
fn scalar_unchecked_modular_wrap_should_discard_invalid_mathematical_conclusions() {
    let maximum = ScalarValue::constant(IntegerType::U32, int(u32::MAX as i128)).expect("max");
    let one = ScalarValue::constant(IntegerType::U32, int(1)).expect("one");
    let wrapped = scalar_binary(
        MirBinaryOp::Add,
        KirArithmeticSemantics::Modular,
        &maximum,
        &one,
    )
    .expect("modular add");

    assert_eq!(wrapped.exact_value(), Some(&int(0)));
    assert_eq!(wrapped.failure(), ScalarFailure::None);
    assert!(wrapped.affine().is_none());

    let broad = scalar_binary(
        MirBinaryOp::Add,
        KirArithmeticSemantics::Modular,
        &ScalarValue::from_interval(
            IntegerType::U32,
            ScalarInterval::new(int(u32::MAX as i128 - 2), int(u32::MAX as i128))
                .expect("interval"),
        )
        .expect("broad"),
        &one,
    )
    .expect("modular broad add");
    assert!(broad.is_unknown());
    assert_eq!(broad.interval().lower(), &int(0));
    assert_eq!(broad.interval().upper(), &int(u32::MAX as i128));
}

#[test]
fn scalar_checked_operations_should_retain_failure_until_range_proves_safety() {
    let maximum = ScalarValue::constant(IntegerType::I32, int(i32::MAX as i128)).expect("max");
    let one = ScalarValue::constant(IntegerType::I32, int(1)).expect("one");
    let overflow = scalar_binary(
        MirBinaryOp::Add,
        KirArithmeticSemantics::Checked,
        &maximum,
        &one,
    )
    .expect("checked add");
    assert_eq!(overflow.failure(), ScalarFailure::Always);

    let safe = scalar_binary(
        MirBinaryOp::Add,
        KirArithmeticSemantics::Checked,
        &ScalarValue::constant(IntegerType::I32, int(5)).expect("five"),
        &ScalarValue::constant(IntegerType::I32, int(7)).expect("seven"),
    )
    .expect("safe add");
    assert_eq!(safe.exact_value(), Some(&int(12)));
    assert_eq!(safe.failure(), ScalarFailure::None);

    let maybe = scalar_binary(
        MirBinaryOp::Mul,
        KirArithmeticSemantics::Checked,
        &ScalarValue::from_interval(
            IntegerType::I32,
            ScalarInterval::new(int(0), int(i32::MAX as i128)).expect("range"),
        )
        .expect("lhs"),
        &ScalarValue::constant(IntegerType::I32, int(2)).expect("two"),
    )
    .expect("maybe multiply");
    assert_eq!(maybe.failure(), ScalarFailure::May);
}

#[test]
fn scalar_strict_comparison_and_branch_refinement_should_be_path_sensitive() {
    let value = ScalarValue::from_interval(
        IntegerType::I32,
        ScalarInterval::new(int(0), int(10)).expect("range"),
    )
    .expect("value");
    let four = ScalarValue::constant(IntegerType::I32, int(4)).expect("four");

    assert_eq!(
        scalar_compare(MirCompareOp::Lt, &value, &four).expect("comparison"),
        BoolLattice::Unknown
    );
    let (taken, not_taken) =
        refine_scalar_comparison(MirCompareOp::Lt, &value, &four).expect("branch refinement");
    assert_eq!(taken.0.interval().lower(), &int(0));
    assert_eq!(taken.0.interval().upper(), &int(3));
    assert_eq!(not_taken.0.interval().lower(), &int(4));
    assert_eq!(not_taken.0.interval().upper(), &int(10));
    assert_eq!(
        scalar_compare(
            MirCompareOp::Lt,
            &ScalarValue::constant(IntegerType::I32, int(-1)).expect("minus one"),
            &four,
        )
        .expect("true comparison"),
        BoolLattice::AlwaysTrue
    );
}

#[test]
fn scalar_widening_narrowing_and_budget_should_be_deterministic() {
    let previous = ScalarValue::from_interval(
        IntegerType::U32,
        ScalarInterval::new(int(0), int(1)).expect("previous"),
    )
    .expect("previous value");
    let next = ScalarValue::from_interval(
        IntegerType::U32,
        ScalarInterval::new(int(0), int(2)).expect("next"),
    )
    .expect("next value");
    let widened = widen_scalar(&previous, &next).expect("widen");
    assert_eq!(widened.interval().lower(), &int(0));
    assert_eq!(widened.interval().upper(), &int(u32::MAX as i128));
    let narrowed = narrow_scalar(&widened, &next).expect("narrow");
    assert_eq!(narrowed.interval(), next.interval());

    let (_, kir) = checked_kir(
        r#"
        export fn sum(n: u32) -> u32 {
          let i: u32 = 0;
          let total: u32 = 0;
          while i < n { total = total + i; i = i + 1; }
          return total;
        }
        "#,
    );
    let function = kir.functions.iter().find(|f| f.name == "sum").expect("sum");
    let standard = analyze_scalar_function(function, ScalarAnalysisConfig::default())
        .expect("standard analysis");
    assert!(!standard.budget().used_wall_clock());
    assert!(standard.steps() <= standard.budget().max_steps());
    assert_eq!(
        standard.narrowing_iterations_run(),
        standard.budget().narrowing_iterations()
    );

    let constrained_config = ScalarAnalysisConfig::with_max_steps(1);
    let constrained =
        analyze_scalar_function(function, constrained_config).expect("constrained analysis");
    assert!(constrained.exhausted());
    assert_eq!(constrained.budget().max_steps(), 1);
    assert!(constrained.values().values().all(ScalarValue::is_unknown));
    for _ in 0..20 {
        assert_eq!(
            analyze_scalar_function(function, constrained_config).expect("repeat"),
            constrained
        );
    }
}

#[test]
fn scalar_analysis_should_materialize_only_locally_proven_facts() {
    let (_, kir) = checked_kir("export fn answer() -> i32 { return 20 + 22; }");
    let function = &kir.functions[0];
    let analysis = analyze_scalar_function(function, ScalarAnalysisConfig::default())
        .expect("scalar analysis");
    let facts = materialize_scalar_facts(function, &analysis, 11).expect("proven facts");

    assert!(facts.facts().len() >= 3);
    assert!(
        facts
            .facts()
            .iter()
            .all(|fact| fact.origin == calckernel::FactOrigin::Proven)
    );
    assert_eq!(
        calckernel::verify_fact_arena(&kir, None, &facts, 11).errors,
        []
    );
    assert!(print_fact_arena(&facts).contains("range("));
}
