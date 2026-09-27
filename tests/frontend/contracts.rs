use calckernel::{
    CheckedAffineTerm, CheckedAffineTermCoefficient, CheckedContractPredicate, ContractEffectKind,
    Declaration, DiagnosticCode, SourceFile, Statement, TokenKind, check, lex, parse,
};

fn source(text: &str) -> SourceFile {
    SourceFile::new("contract.ck", text)
}

#[test]
fn contract_lexer_should_reserve_structural_contract_keywords() {
    let result = lex(&source(
        "unsafe contract requires effects none export fn read write noalias aligned multiple_of",
    ));
    let kinds = result
        .tokens
        .iter()
        .map(|token| token.kind)
        .collect::<Vec<_>>();

    assert_eq!(
        kinds,
        vec![
            TokenKind::Unsafe,
            TokenKind::Contract,
            TokenKind::Requires,
            TokenKind::Effects,
            TokenKind::None,
            TokenKind::Export,
            TokenKind::Fn,
            TokenKind::Identifier,
            TokenKind::Identifier,
            TokenKind::Identifier,
            TokenKind::Identifier,
            TokenKind::Identifier,
            TokenKind::Eof,
        ]
    );
}

#[test]
fn contract_lexer_should_report_new_keyword_utf16_spans_after_non_bmp_text() {
    let result = lex(&source("😀 unsafe contract requires effects none"));
    let tokens = result
        .tokens
        .iter()
        .filter(|token| token.kind != TokenKind::Eof)
        .map(|token| (token.kind, token.start, token.end, token.line, token.column))
        .collect::<Vec<_>>();

    assert_eq!(
        tokens,
        vec![
            (TokenKind::Unsafe, 3, 9, 1, 4),
            (TokenKind::Contract, 10, 18, 1, 11),
            (TokenKind::Requires, 19, 27, 1, 20),
            (TokenKind::Effects, 28, 35, 1, 29),
            (TokenKind::None, 36, 40, 1, 37),
        ]
    );
}

#[test]
fn contract_parser_should_preserve_unsafe_contract_and_statement_block() {
    let result = parse(&source(
        r#"
        export unsafe fn copy(x: slice<i32>, y: slice<i32>, n: u32) -> void
        contract {
          requires n <= x.len && n <= y.len;
          requires noalias(x, y);
          requires aligned(x.data, 32);
          effects read(x), write(y);
        }
        { y[0] = x[0]; }

        fn main(p: ptr<i32>) -> void {
          let x: slice<i32> = slice(p, 0);
          unsafe { copy(x, x, 0); }
        }
        "#,
    ));

    assert_eq!(result.diagnostics, []);
    let Declaration::Function(copy) = &result.ast.declarations[0] else {
        panic!("expected copy function");
    };
    assert!(copy.is_unsafe);
    let contract = copy.contract.as_ref().expect("unsafe contract");
    assert_eq!(contract.requirements.len(), 3);
    assert_eq!(contract.effects.as_ref().expect("effects").items.len(), 2);
    let Declaration::Function(main) = &result.ast.declarations[1] else {
        panic!("expected main function");
    };
    assert!(matches!(main.body.statements[1], Statement::Unsafe(_)));
}

#[test]
fn contract_parser_should_preserve_effects_none() {
    let result = parse(&source(
        r#"
        unsafe fn inspect(x: slice<i32>) -> void
        contract { requires x.len >= 0; effects none; }
        { return; }
        "#,
    ));

    assert_eq!(result.diagnostics, []);
    let Declaration::Function(function) = &result.ast.declarations[0] else {
        panic!("expected function");
    };
    let effects = function
        .contract
        .as_ref()
        .and_then(|contract| contract.effects.as_ref())
        .expect("effects none");
    assert!(effects.is_none);
    assert_eq!(effects.items, []);
}

#[test]
fn contract_parser_should_reject_reversed_function_modifiers_once() {
    let result = parse(&source(
        "unsafe export fn bad(n: u32) -> void contract { requires n > 0; } { return; }",
    ));

    assert_eq!(result.ast.declarations.len(), 1);
    assert_eq!(result.diagnostics.len(), 1);
    assert_eq!(result.diagnostics[0].code, DiagnosticCode::Ck2014);
    assert_eq!(result.diagnostics[0].span.start.offset, 7);
    assert_eq!(
        result.diagnostics[0].message,
        "Function modifiers must appear in '[export] [unsafe] fn' order."
    );
}

#[test]
fn contract_parser_should_reject_isolated_contract_once() {
    let result = parse(&source("contract { requires 1 == 1; effects none; }"));

    assert_eq!(result.ast.declarations, []);
    assert_eq!(result.diagnostics.len(), 1);
    assert_eq!(result.diagnostics[0].code, DiagnosticCode::Ck2014);
    assert_eq!(result.diagnostics[0].span.start.offset, 0);
    assert_eq!(
        result.diagnostics[0].message,
        "A contract may appear only after an unsafe function signature."
    );
}

#[test]
fn contract_parser_should_reject_top_level_unsafe_block_once() {
    let result = parse(&source("unsafe { return; }"));

    assert_eq!(result.ast.declarations, []);
    assert_eq!(result.diagnostics.len(), 1);
    assert_eq!(result.diagnostics[0].code, DiagnosticCode::Ck2014);
    assert_eq!(result.diagnostics[0].span.start.offset, 0);
    assert_eq!(
        result.diagnostics[0].message,
        "An unsafe block may appear only inside a function body."
    );
}

#[test]
fn contract_checker_should_build_typed_affine_and_predicate_metadata() {
    let result = check(&source(
        r#"
        export unsafe fn kernel(x: slice<i32>, y: slice<i32>, n: u32) -> void
        contract {
          requires n + 2 <= x.len && n <= y.len;
          requires multiple_of(n, 4);
          requires noalias(x, y);
          requires aligned(y.data, 32);
          effects read(x), write(y);
        }
        { y[0] = x[0]; }
        "#,
    ));

    assert_eq!(result.diagnostics, []);
    let kernel = result
        .checked_program
        .function_map
        .get("kernel")
        .expect("kernel metadata");
    let contract = kernel.contract.as_ref().expect("checked contract");
    assert!(kernel.is_unsafe);
    assert!(matches!(
        contract.predicates[1],
        CheckedContractPredicate::MultipleOf { .. }
    ));
    assert_eq!(
        contract.effects.as_ref().expect("effect ceiling").items,
        vec![
            ("x".to_string(), ContractEffectKind::Read),
            ("y".to_string(), ContractEffectKind::Write),
        ]
    );
}

#[test]
fn contract_checker_should_normalize_unbounded_affine_terms_deterministically() {
    let result = check(&source(
        r#"
        unsafe fn kernel(x: slice<i32>, n: u32) -> void
        contract {
          requires 3 + x.len + 2 * n - 8 <= x.len;
          requires multiple_of(n, 184467440737095516170);
        }
        { return; }
        "#,
    ));

    assert_eq!(result.diagnostics, []);
    let contract = result.checked_program.function_map["kernel"]
        .contract
        .as_ref()
        .expect("checked contract");
    let CheckedContractPredicate::Comparison { left, .. } = &contract.predicates[0] else {
        panic!("expected comparison");
    };
    assert_eq!(
        left.terms,
        vec![
            CheckedAffineTermCoefficient {
                term: CheckedAffineTerm::Parameter("n".to_string()),
                coefficient: "2".to_string(),
            },
            CheckedAffineTermCoefficient {
                term: CheckedAffineTerm::SliceLength("x".to_string()),
                coefficient: "1".to_string(),
            },
        ]
    );
    assert_eq!(left.constant, "-5");
    let CheckedContractPredicate::MultipleOf { modulus, .. } = &contract.predicates[1] else {
        panic!("expected multiple_of");
    };
    assert_eq!(modulus, "184467440737095516170");
}

#[test]
fn contract_checker_should_normalize_effect_lattice_in_parameter_order() {
    let result = check(&source(
        r#"
        unsafe fn kernel(x: slice<i32>, y: slice<i32>, z: slice<i32>) -> void
        contract {
          requires x.len >= 0;
          effects write(z), read(x);
        }
        { return; }
        "#,
    ));

    assert_eq!(result.diagnostics, []);
    let effects = result.checked_program.function_map["kernel"]
        .contract
        .as_ref()
        .and_then(|contract| contract.effects.as_ref())
        .expect("checked effects");
    assert_eq!(
        effects.items,
        vec![
            ("x".to_string(), ContractEffectKind::Read),
            ("y".to_string(), ContractEffectKind::None),
            ("z".to_string(), ContractEffectKind::Write),
        ]
    );
}

#[test]
fn contract_checker_should_reject_duplicate_effect_target_at_second_name() {
    let result = check(&source(
        r#"
        unsafe fn kernel(x: slice<i32>) -> void
        contract {
          requires x.len >= 0;
          effects read(x), write(x);
        }
        { return; }
        "#,
    ));

    let diagnostics = result
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == DiagnosticCode::Ck2015)
        .collect::<Vec<_>>();
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(
        diagnostics[0].message,
        "An effect target can appear only once in a ceiling."
    );
    assert_eq!(diagnostics[0].span.start.column, 34);
}

#[test]
fn contract_checker_should_expand_effects_none_to_all_slice_parameters() {
    let result = check(&source(
        r#"
        unsafe fn kernel(x: slice<i32>, y: slice<i32>) -> void
        contract { requires x.len >= 0; effects none; }
        { return; }
        "#,
    ));

    assert_eq!(result.diagnostics, []);
    let effects = result.checked_program.function_map["kernel"]
        .contract
        .as_ref()
        .and_then(|contract| contract.effects.as_ref())
        .expect("checked effects");
    assert!(effects.is_none);
    assert_eq!(
        effects.items,
        vec![
            ("x".to_string(), ContractEffectKind::None),
            ("y".to_string(), ContractEffectKind::None),
        ]
    );
}

#[test]
fn contract_checker_should_require_explicit_unsafe_call_boundary() {
    let result = check(&source(
        r#"
        unsafe fn dangerous(n: u32) -> u32
        contract { requires n < 10; }
        { return n + 1; }

        fn main() -> i32 { return dangerous(1); }
        "#,
    ));

    let diagnostic = result
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == DiagnosticCode::Ck2014)
        .expect("unsafe-call diagnostic");
    assert_eq!(
        diagnostic.message,
        "Call to unsafe function 'dangerous' requires an explicit unsafe block."
    );
}

#[test]
fn contract_checker_should_accept_nested_unsafe_call_expressions() {
    let result = check(&source(
        r#"
        unsafe fn value(n: u32) -> i32
        contract { requires n == 1; }
        { return 7; }

        fn main() -> i32 {
          unsafe { return value(1); }
        }
        "#,
    ));

    assert_eq!(result.diagnostics, []);
}

#[test]
fn contract_checker_should_reject_unsafe_main_without_creating_entry() {
    let result = check(&source(
        r#"
        unsafe fn main() -> void
        contract { requires 1 == 1; }
        { return; }
        "#,
    ));

    assert!(result.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == DiagnosticCode::Ck2014
            && diagnostic.message == "Program entry 'main' cannot be unsafe or contracted."
    }));
    assert_eq!(result.checked_program.entry, None);
}

#[test]
fn contract_checker_should_reject_safe_contracted_main_without_creating_entry() {
    let result = check(&source(
        r#"
        fn main() -> void
        contract { requires 1 == 1; effects none; }
        { return; }
        "#,
    ));

    assert_eq!(
        result
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == DiagnosticCode::Ck2014)
            .count(),
        1
    );
    assert_eq!(result.checked_program.entry, None);
}

#[test]
fn contract_checker_should_reject_safe_contract_and_unsafe_without_requires() {
    let result = check(&source(
        r#"
        fn safe(x: slice<i32>) -> void
        contract { requires x.len > 0; effects read(x); }
        { return; }

        unsafe fn missing(x: slice<i32>) -> void
        contract { effects read(x); }
        { return; }
        "#,
    ));
    let messages = result
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == DiagnosticCode::Ck2014)
        .map(|diagnostic| diagnostic.message.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        messages,
        vec![
            "A safe function cannot declare a contract or effects ceiling.",
            "An unsafe function contract requires at least one 'requires' clause.",
        ]
    );
}

#[test]
fn contract_checker_should_reject_non_affine_and_ill_typed_predicates() {
    let result = check(&source(
        r#"
        unsafe fn bad(x: slice<i32>, y: slice<i32>, n: u32) -> void
        contract {
          requires n * x.len < y.len;
          requires n < x.data;
          requires noalias(x, n);
          requires aligned(x.data, 3);
          requires n < 2 || n > 8;
          effects read(n);
        }
        { return; }
        "#,
    ));

    assert_eq!(
        result
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == DiagnosticCode::Ck2015)
            .count(),
        6
    );
}

#[test]
fn contract_checker_should_reject_each_closed_dsl_violation_once() {
    for (name, declaration) in [
        (
            "non_bool",
            "unsafe fn bad(n: u32) -> void contract { requires true; } { return; }",
        ),
        (
            "ordinary_call",
            "fn helper(n: u32) -> bool { return true; } unsafe fn bad(n: u32) -> void contract { requires helper(n); } { return; }",
        ),
        (
            "negation",
            "unsafe fn bad(n: u32) -> void contract { requires !(n == 0); } { return; }",
        ),
        (
            "memory_load",
            "unsafe fn bad(x: slice<i32>) -> void contract { requires x[0] > 0; } { return; }",
        ),
        (
            "mixed_integer_types",
            "unsafe fn bad(i: i32, n: u32) -> void contract { requires i < n; } { return; }",
        ),
        (
            "oversized_alignment",
            "unsafe fn bad(p: ptr<i32>) -> void contract { requires aligned(p, 2147483649); } { return; }",
        ),
        (
            "zero_modulus",
            "unsafe fn bad(n: u32) -> void contract { requires multiple_of(n, 0); } { return; }",
        ),
    ] {
        let result = check(&source(declaration));
        let diagnostics = result
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == DiagnosticCode::Ck2015)
            .collect::<Vec<_>>();
        assert_eq!(
            diagnostics.len(),
            1,
            "case: {name}: {:?}",
            result.diagnostics
        );
    }
}

#[test]
fn contract_checker_should_require_unsafe_block_inside_unsafe_function() {
    let result = check(&source(
        r#"
        unsafe fn callee(n: u32) -> u32
        contract { requires n > 0; }
        { return n; }

        unsafe fn caller(n: u32) -> u32
        contract { requires n > 0; }
        { return callee(n); }
        "#,
    ));

    let diagnostics = result
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == DiagnosticCode::Ck2014)
        .collect::<Vec<_>>();
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(
        diagnostics[0].message,
        "Call to unsafe function 'callee' requires an explicit unsafe block."
    );
}

#[test]
fn contract_checker_should_not_suppress_unrelated_errors_inside_unsafe_block() {
    let result = check(&source(
        r#"
        unsafe fn value(n: u32) -> i32
        contract { requires n > 0; }
        { return 1; }

        fn caller() -> i32 {
          unsafe {
            let invalid: i32 = true;
            return value(1);
          }
        }
        "#,
    ));

    assert_eq!(
        result
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == DiagnosticCode::Ck2004)
            .count(),
        1
    );
    assert!(
        !result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == DiagnosticCode::Ck2014)
    );
}

#[test]
fn ck2016_effects_none_should_allow_no_external_memory_but_keep_other_summary_flags() {
    let result = check(&source(
        r#"
        unsafe fn observable(n: i32) -> void
        contract { requires n >= 0; effects none; }
        { print_i32(n + 1); }
        "#,
    ));

    assert_eq!(
        result
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == DiagnosticCode::Ck2016)
            .count(),
        0
    );
    let summary = &result.checked_program.effect_summaries["observable"];
    assert!(summary.runtime_effect);
    assert!(summary.may_fail);
    assert_eq!(
        summary.effect(calckernel::EffectTarget::All),
        calckernel::MemoryEffect::None
    );
}

#[test]
fn ck2016_effect_ceiling_should_accept_exact_read_write_and_readwrite_accesses() {
    let result = check(&source(
        r#"
        unsafe fn kernel(x: slice<i32>, y: slice<i32>, z: slice<i32>) -> i32
        contract {
          requires x.len > 0 && y.len > 0 && z.len > 0;
          effects read(x), write(y), readwrite(z);
        }
        { y[0] = x[0]; z[0] = z[0] + 1; return x[0]; }
        "#,
    ));

    assert!(
        !result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == DiagnosticCode::Ck2016),
        "{:?}",
        result.diagnostics
    );
}

#[test]
fn ck2016_effect_ceiling_should_reject_each_underdeclared_slice_access() {
    for (name, body, expected) in [
        (
            "write",
            "x[0] = 1;",
            "does not allow write access to slice parameter 'x'",
        ),
        (
            "read",
            "let value: i32 = x[0];",
            "does not allow read access to slice parameter 'x'",
        ),
        (
            "readwrite",
            "x[0] = x[0] + 1;",
            "does not allow readwrite access to slice parameter 'x'",
        ),
    ] {
        let text = format!(
            "unsafe fn kernel(x: slice<i32>) -> void contract {{ requires x.len > 0; effects none; }} {{ {body} }}"
        );
        let result = check(&source(&text));
        let diagnostics = result
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == DiagnosticCode::Ck2016)
            .collect::<Vec<_>>();
        assert_eq!(
            diagnostics.len(),
            1,
            "case {name}: {:?}",
            result.diagnostics
        );
        assert!(diagnostics[0].message.contains(expected));
    }
}

#[test]
fn ck2016_effect_ceiling_should_map_subslice_and_transitive_callee_back_to_parameter() {
    let accepted = check(&source(
        r#"
        fn write_first(items: slice<i32>) -> void { items[0] = 1; }
        unsafe fn wrapper(items: slice<i32>) -> void
        contract { requires items.len > 1; effects write(items); }
        { let tail: slice<i32> = items[1..items.len]; write_first(tail); }
        "#,
    ));
    assert!(
        !accepted
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == DiagnosticCode::Ck2016),
        "{:?}",
        accepted.diagnostics
    );

    let rejected = check(&source(
        r#"
        fn write_first(items: slice<i32>) -> void { items[0] = 1; }
        unsafe fn wrapper(items: slice<i32>) -> void
        contract { requires items.len > 0; effects read(items); }
        { write_first(items); }
        "#,
    ));
    assert_eq!(
        rejected
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == DiagnosticCode::Ck2016)
            .count(),
        1
    );
}

#[test]
fn ck2016_effect_ceiling_should_reject_raw_pointer_and_unknown_all_access() {
    let result = check(&source(
        r#"
        unsafe fn kernel(x: slice<i32>, raw: ptr<i32>) -> void
        contract { requires x.len > 0; effects read(x); }
        { raw[0] = x[0]; }
        "#,
    ));
    let diagnostic = result
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == DiagnosticCode::Ck2016)
        .expect("CK2016");

    assert!(diagnostic.message.contains("conservative write access"));
}
