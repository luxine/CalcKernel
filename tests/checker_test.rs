use calckernel::{
    CalcKernelType, DiagnosticCode, Expression, PrimitiveTypeName, Scope, SourceFile, Statement,
    VariableSymbol, check, get_expr_type, get_field_info, get_function_info, get_let_type,
    get_struct_info,
};

fn check_source(text: &str) -> calckernel::CheckResult {
    check(&SourceFile::new("test.ck", text))
}

fn messages_of(text: &str) -> Vec<String> {
    check_source(text)
        .diagnostics
        .into_iter()
        .map(|diagnostic| diagnostic.message)
        .collect()
}

#[test]
fn check_should_accept_valid_pricing_style_program() {
    let result = check_source(
        r#"
      struct Item {
        price: i64;
        qty: i64;
        tax_rate_ppm: i64;
      }

      export fn tax(base: i64, ppm: i64) -> i64 {
        return base * ppm / 1000000;
      }

      export fn calc(items: ptr<Item>, len: i32, out: ptr<i64>) -> i32 {
        let i: i32 = 0;
        while i < len {
          let subtotal: i64 = items[i].price * items[i].qty;
          if subtotal > 0 {
            out[i] = subtotal + tax(subtotal, items[i].tax_rate_ppm);
          } else {
            out[i] = 0;
          }
          i = i + 1;
        }
        return 0;
      }
    "#,
    );

    assert_eq!(result.diagnostics, []);
    assert!(result.checked_program.function_map.contains_key("calc"));
    assert!(result.checked_program.struct_map.contains_key("Item"));
    assert!(
        result
            .typed_ast
            .expression_types
            .values()
            .any(|type_node| matches!(
                type_node,
                calckernel::CalcKernelType::Primitive(calckernel::PrimitiveTypeName::Bool)
            ))
    );
}

#[test]
fn check_should_expose_typescript_compatible_symbol_lookup_helpers() {
    let result = check_source(
        r#"
      struct Item {
        price: i64;
        qty: i32;
      }

      export fn total(items: ptr<Item>) -> i64 {
        return items[0].price;
      }
    "#,
    );

    assert_eq!(result.diagnostics, []);
    let program = &result.checked_program;
    assert_eq!(
        get_struct_info(program, "Item").expect("Item struct").name,
        "Item"
    );
    assert_eq!(
        get_field_info(program, "Item", "price")
            .expect("price field")
            .name,
        "price"
    );
    assert_eq!(
        get_function_info(program, "total")
            .expect("total function")
            .name,
        "total"
    );
    assert!(get_struct_info(program, "Missing").is_none());
    assert!(get_field_info(program, "Item", "missing").is_none());
    assert!(get_function_info(program, "missing").is_none());
}

#[test]
fn scope_should_expose_typescript_compatible_declare_and_lookup_behavior() {
    let mut scope = Scope::default();
    let outer = VariableSymbol {
        name: "value".to_string(),
        type_node: CalcKernelType::Primitive(PrimitiveTypeName::I64),
    };

    assert!(scope.declare(outer.clone()));
    assert!(!scope.declare(outer.clone()));
    assert_eq!(scope.lookup("value"), Some(&outer));
    assert_eq!(scope.lookup("missing"), None);

    let inner = VariableSymbol {
        name: "value".to_string(),
        type_node: CalcKernelType::Primitive(PrimitiveTypeName::I32),
    };
    scope.push();
    assert!(scope.declare(inner.clone()));
    assert_eq!(scope.lookup("value"), Some(&inner));
    scope.pop();
    assert_eq!(scope.lookup("value"), Some(&outer));
}

#[test]
fn check_should_report_unknown_variable_with_ck2001() {
    let result = check_source(
        r#"
      export fn bad() -> i32 {
        return missing;
      }
    "#,
    );

    assert!(result.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == DiagnosticCode::Ck2001
            && diagnostic.message == "Unknown variable 'missing'."
            && diagnostic.file_name == "test.ck"
            && diagnostic.line == 3
            && diagnostic.column == 16
    }));
}

#[test]
fn check_should_report_return_type_mismatch() {
    assert!(
        messages_of(
            r#"
      export fn bad() -> i32 {
        return true;
      }
    "#
        )
        .contains(&"Return type mismatch: expected i32 but got bool.".to_string())
    );
}

#[test]
fn check_should_accept_explicit_i32_and_u32_to_f64_builtins() {
    let result = check_source(
        r#"
      export fn from_i32(n: i32) -> f64 {
        let x: f64 = i32_to_f64(n);
        return x + 1.0;
      }

      export fn from_u32(n: u32) -> f64 {
        return u32_to_f64(n);
      }
    "#,
    );

    assert_eq!(result.diagnostics, []);
}

#[test]
fn check_should_reject_strict_f64_violations() {
    assert!(
        messages_of("export fn bad() -> f64 { let x: f64 = 1; return 1.0; }")
            .contains(&"Cannot initialize 'x': expected f64 but got i32.".to_string())
    );
    assert!(
        messages_of("export fn bad(a: f64, b: i64) -> f64 { return a + b; }").contains(
            &"Arithmetic operator '+' requires integer operands of the same type.".to_string()
        )
    );
    assert!(
        messages_of("export fn bad(a: f64, b: f64) -> f64 { return a % b; }")
            .contains(&"Arithmetic operator '%' does not support f64 operands.".to_string())
    );
}

#[test]
fn check_should_expose_expression_and_let_types_for_mir_lowering() {
    let result = check_source(
        r#"
      struct Item {
        price: i64;
        qty: i64;
      }

      export fn add(a: i64, b: i64) -> i64 {
        return a + b;
      }

      export fn calc(item: ptr<Item>) -> i64 {
        let subtotal: i64 = item[0].price + add(1, 2);
        return subtotal;
      }
    "#,
    );

    assert_eq!(result.diagnostics, []);
    let calc = result
        .checked_program
        .function_map
        .get("calc")
        .expect("calc function info");
    let Statement::Let(let_statement) = &calc.declaration.body.statements[0] else {
        panic!("expected let statement");
    };
    let Expression::Binary { left, right, .. } = &let_statement.initializer else {
        panic!("expected binary initializer");
    };

    assert_eq!(
        get_let_type(&result.checked_program, let_statement),
        Some(&calckernel::CalcKernelType::Primitive(
            calckernel::PrimitiveTypeName::I64
        ))
    );
    assert_eq!(
        get_expr_type(&result.checked_program, &let_statement.initializer),
        Some(&calckernel::CalcKernelType::Primitive(
            calckernel::PrimitiveTypeName::I64
        ))
    );
    assert_eq!(
        get_expr_type(&result.checked_program, left),
        Some(&calckernel::CalcKernelType::Primitive(
            calckernel::PrimitiveTypeName::I64
        ))
    );
    assert_eq!(
        get_expr_type(&result.checked_program, right),
        Some(&calckernel::CalcKernelType::Primitive(
            calckernel::PrimitiveTypeName::I64
        ))
    );
}

#[test]
fn check_should_reject_break_and_continue_outside_while_with_ck2009() {
    let result = check_source(
        r#"
      export fn bad_break() -> i32 {
        break;
        return 0;
      }

      export fn bad_continue() -> i32 {
        continue;
        return 0;
      }
    "#,
    );
    let diagnostics = result
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code.to_string() == "CK2009")
        .map(|diagnostic| {
            (
                diagnostic.message.as_str(),
                diagnostic.line,
                diagnostic.column,
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(
        diagnostics,
        vec![
            ("'break' can only be used inside a while loop.", 3, 9),
            ("'continue' can only be used inside a while loop.", 8, 9),
        ]
    );
}

#[test]
fn check_should_report_unreachable_after_non_fallthrough_with_ck2010() {
    let result = check_source(
        r#"
      export fn after_return() -> i32 {
        return 0;
        return 1;
      }

      export fn after_break() -> i32 {
        while true {
          break;
          continue;
        }
        return 0;
      }

      export fn after_continue() -> i32 {
        while true {
          continue;
          break;
        }
        return 0;
      }
    "#,
    );
    let unreachable = result
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code.to_string() == "CK2010")
        .map(|diagnostic| (diagnostic.line, diagnostic.column))
        .collect::<Vec<_>>();

    assert_eq!(unreachable, vec![(4, 9), (10, 11), (18, 11)]);
}

#[test]
fn check_should_combine_if_branch_flow_inside_while() {
    let result = check_source(
        r#"
      export fn branch_flow(flag: bool) -> i32 {
        while true {
          if flag {
            break;
          } else {
            continue;
          }
          let unreachable: i32 = 1;
        }

        while true {
          if flag {
            break;
          }
          let reachable: i32 = 1;
          break;
        }
        return 0;
      }
    "#,
    );
    let unreachable = result
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code.to_string() == "CK2010")
        .map(|diagnostic| (diagnostic.line, diagnostic.column))
        .collect::<Vec<_>>();

    assert_eq!(unreachable, vec![(9, 11)]);
}

#[test]
fn check_should_conservatively_require_a_return_after_while_true() {
    let result = check_source(
        r#"
      export fn loop_only() -> i32 {
        while true {
          continue;
        }
      }
    "#,
    );

    assert!(
        result
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.message == "Missing return in function 'loop_only'." })
    );
}

#[test]
fn check_should_accept_void_fallthrough_and_empty_return() {
    let result = check_source(
        r#"
      fn no_op() -> void {}

      export fn maybe_stop(stop: bool) -> void {
        if stop {
          return;
        }
        no_op();
      }
    "#,
    );

    assert_eq!(result.diagnostics, []);
}

#[test]
fn check_should_reject_void_in_value_type_positions_with_ck2011() {
    let result = check_source(
        r#"
      struct BadField { value: void; }

      fn no_op() -> void {}
      fn bad_param(value: void) -> void {}
      fn bad_pointer(value: ptr<void>) -> void {}
      fn bad_local() -> void {
        let value: void = no_op();
      }
      fn take_i32(value: i32) -> void {}
      fn bad_argument() -> void {
        take_i32(no_op());
      }
    "#,
    );

    let void_errors = result
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code.to_string() == "CK2011")
        .map(|diagnostic| diagnostic.message.as_str())
        .collect::<Vec<_>>();
    assert!(void_errors.contains(&"Void is only allowed as a function return type."));
    assert!(void_errors.contains(&"Void is not allowed as a pointer element type."));
    assert!(void_errors.contains(&"A void call cannot be used where a value is required."));
    assert!(void_errors.len() >= 5, "{void_errors:?}");
}

#[test]
fn check_should_reject_mismatched_void_returns_with_ck2011() {
    let result = check_source(
        r#"
      fn bad_void() -> void { return 1; }
      fn bad_value() -> i32 { return; }
    "#,
    );

    let errors = result
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code.to_string() == "CK2011")
        .map(|diagnostic| diagnostic.message.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        errors,
        vec![
            "A void function cannot return a value.",
            "A non-void function must return a value.",
        ]
    );
}

#[test]
fn check_should_accept_only_void_calls_as_statements() {
    let accepted = check_source(
        r#"
      fn no_op() -> void {}
      export fn run() -> void { no_op(); }
    "#,
    );
    assert_eq!(accepted.diagnostics, []);

    let rejected = check_source(
        r#"
      fn value() -> i32 { return 1; }
      export fn run() -> void { value(); }
    "#,
    );
    assert!(rejected.diagnostics.iter().any(|diagnostic| {
        diagnostic.code.to_string() == "CK2011"
            && diagnostic.message == "Only a void call may be used as a standalone statement."
    }));
}

#[test]
fn check_should_reject_void_call_in_value_contexts() {
    let result = check_source(
        r#"
      fn no_op() -> void {}
      fn take(value: i32) -> i32 { return value; }
      fn initializer() -> i32 { let x: i32 = no_op(); return 0; }
      fn returned() -> i32 { return no_op(); }
      fn argument() -> i32 { return take(no_op()); }
      fn arithmetic() -> i32 { return no_op() + 1; }
      fn comparison() -> bool { return no_op() == no_op(); }
    "#,
    );

    let void_value_errors = result
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic.code.to_string() == "CK2011"
                && diagnostic.message == "A void call cannot be used where a value is required."
        })
        .count();
    assert_eq!(void_value_errors, 6);
}

#[test]
fn check_should_preserve_unreachable_analysis_after_empty_return() {
    let result = check_source(
        r#"
      export fn stop() -> void {
        return;
        no_op();
      }
      fn no_op() -> void {}
    "#,
    );

    assert!(result.diagnostics.iter().any(|diagnostic| {
        diagnostic.code.to_string() == "CK2010"
            && diagnostic.message == "Unreachable statement."
            && diagnostic.line == 4
    }));
}
