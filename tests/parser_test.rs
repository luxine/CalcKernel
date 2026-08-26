use calckernel::{Declaration, Expression, SourceFile, Statement, TypeNode, parse};

fn parse_source(text: &str) -> calckernel::ParseResult {
    parse(&SourceFile::new("test.ck", text))
}

fn parse_return_expression(text: &str) -> Expression {
    let result = parse_source(text);
    assert_eq!(result.diagnostics, []);

    let declaration = result
        .ast
        .declarations
        .iter()
        .find_map(|declaration| match declaration {
            Declaration::Function(function) => Some(function),
            Declaration::Struct(_) => None,
        })
        .expect("expected function declaration");
    let Statement::Return(statement) = &declaration.body.statements[0] else {
        panic!("expected return statement");
    };

    statement.value.clone()
}

#[test]
fn parse_should_parse_struct_declarations_with_typed_fields() {
    let source = SourceFile::new(
        "test.ck",
        r#"
      struct Item {
        price: i64;
        qty: i32;
      }
    "#,
    );
    let result = parse(&source);

    assert_eq!(result.diagnostics, []);
    let Declaration::Struct(struct_decl) = &result.ast.declarations[0] else {
        panic!("expected struct declaration");
    };
    assert_eq!(struct_decl.name.name, "Item");
    assert_eq!(struct_decl.fields[0].name.name, "price");
    assert_eq!(
        struct_decl.fields[0].type_node,
        TypeNode::Primitive {
            name: "i64".to_string(),
            span: struct_decl.fields[0].type_node.span(),
        }
    );
    assert_eq!(struct_decl.fields[1].name.name, "qty");
    assert_eq!(struct_decl.span.start.line, 2);
}

#[test]
fn parse_should_parse_f64_primitive_types() {
    let result = parse_source(
        r#"
      export fn scale(value: f64) -> f64 {
        return value;
      }
    "#,
    );

    assert_eq!(result.diagnostics, []);
    let Declaration::Function(function) = &result.ast.declarations[0] else {
        panic!("expected function declaration");
    };
    assert!(matches!(
        function.params[0].type_node,
        TypeNode::Primitive { ref name, .. } if name == "f64"
    ));
    assert!(matches!(
        function.return_type,
        TypeNode::Primitive { ref name, .. } if name == "f64"
    ));
}

#[test]
fn parse_should_parse_export_functions_params_return_type_and_core_statements() {
    let result = parse_source(
        r#"
      export fn calc(items: ptr<Item>, len: i32, out: ptr<i64>) -> i32 {
        let i: i32 = 0;
        while i < len {
          out[i] = items[i].price + compute(i, len);
          i = i + 1;
        }
        return 0;
      }
    "#,
    );

    assert_eq!(result.diagnostics, []);
    let Declaration::Function(function) = &result.ast.declarations[0] else {
        panic!("expected function declaration");
    };
    assert!(function.exported);
    assert_eq!(function.name.name, "calc");
    assert_eq!(function.params[0].name.name, "items");
    assert!(matches!(
        function.params[0].type_node,
        TypeNode::Pointer { .. }
    ));
    assert!(matches!(
        function.params[1].type_node,
        TypeNode::Primitive { ref name, .. } if name == "i32"
    ));
    assert!(matches!(
        function.params[2].type_node,
        TypeNode::Pointer { ref element_type, .. }
        if matches!(element_type.as_ref(), TypeNode::Primitive { name, .. } if name == "i64")
    ));
    assert!(matches!(
        function.return_type,
        TypeNode::Primitive { ref name, .. } if name == "i32"
    ));
    assert!(matches!(function.body.statements[0], Statement::Let(_)));
    assert!(matches!(function.body.statements[1], Statement::While(_)));
    assert!(matches!(function.body.statements[2], Statement::Return(_)));

    let Statement::While(while_statement) = &function.body.statements[1] else {
        panic!("expected while statement");
    };
    let Statement::Assignment(assignment) = &while_statement.body.statements[0] else {
        panic!("expected assignment statement");
    };
    assert!(matches!(assignment.target, Expression::Index { .. }));
    assert!(matches!(
        assignment.value,
        Expression::Binary { ref operator, .. } if operator == "+"
    ));
}

#[test]
fn parse_should_preserve_multiplication_before_addition() {
    let expression = parse_return_expression(
        r#"
        export fn calc() -> i32 {
          return 1 + 2 * 3;
        }
      "#,
    );

    let Expression::Binary {
        operator,
        left,
        right,
        ..
    } = expression
    else {
        panic!("expected binary expression");
    };
    assert_eq!(operator, "+");
    assert!(matches!(
        left.as_ref(),
        Expression::IntegerLiteral { text, .. } if text == "1"
    ));
    assert!(matches!(
        right.as_ref(),
        Expression::Binary { operator, .. } if operator == "*"
    ));
}

#[test]
fn parse_should_parse_parenthesized_addition_before_multiplication() {
    let expression = parse_return_expression(
        r#"
        export fn calc() -> i32 {
          return (1 + 2) * 3;
        }
      "#,
    );

    let Expression::Binary {
        operator,
        left,
        right,
        ..
    } = expression
    else {
        panic!("expected binary expression");
    };
    assert_eq!(operator, "*");
    assert!(matches!(left.as_ref(), Expression::Parenthesized { .. }));
    assert!(matches!(
        right.as_ref(),
        Expression::IntegerLiteral { text, .. } if text == "3"
    ));
}

#[test]
fn parse_should_parse_field_and_index_access_above_multiplication() {
    let expression = parse_return_expression(
        r#"
        struct Item {
          price: i64;
          qty: i64;
        }

        export fn calc(items: ptr<Item>) -> i64 {
          return items[0].price * items[0].qty;
        }
      "#,
    );

    let Expression::Binary {
        operator,
        left,
        right,
        ..
    } = expression
    else {
        panic!("expected binary expression");
    };
    assert_eq!(operator, "*");
    assert!(matches!(
        left.as_ref(),
        Expression::Field { field, .. } if field.name == "price"
    ));
    assert!(matches!(
        right.as_ref(),
        Expression::Field { field, .. } if field.name == "qty"
    ));
}

#[test]
fn parse_should_parse_logical_or_below_logical_and() {
    let expression = parse_return_expression(
        r#"
        export fn calc(a: bool, b: bool, c: bool) -> bool {
          return a || b && c;
        }
      "#,
    );

    let Expression::Binary {
        operator,
        left,
        right,
        ..
    } = expression
    else {
        panic!("expected binary expression");
    };
    assert_eq!(operator, "||");
    assert!(matches!(
        left.as_ref(),
        Expression::Identifier { name, .. } if name == "a"
    ));
    assert!(matches!(
        right.as_ref(),
        Expression::Binary { operator, .. } if operator == "&&"
    ));
}

#[test]
fn parse_should_add_line_and_column_diagnostics_for_parser_errors() {
    let result = parse_source(
        r#"
      export fn bad() -> i32 {
        let x: i32 = 1
        return x;
      }
    "#,
    );

    assert!(result.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == calckernel::DiagnosticCode::Ck1001
            && diagnostic.message == "Expected ';' after let statement."
            && diagnostic.file_name == "test.ck"
            && diagnostic.line == 4
            && diagnostic.column == 9
    }));
}

#[test]
fn parse_should_parse_break_and_continue_statements_with_spans() {
    let result = parse_source(
        r#"
      export fn control() -> i32 {
        while true {
          break;
          continue;
        }
        return 0;
      }
    "#,
    );

    assert_eq!(result.diagnostics, []);
    let Declaration::Function(function) = &result.ast.declarations[0] else {
        panic!("expected function declaration");
    };
    let Statement::While(while_statement) = &function.body.statements[0] else {
        panic!("expected while statement");
    };
    let break_statement = &while_statement.body.statements[0];
    let continue_statement = &while_statement.body.statements[1];

    assert_eq!(
        format!("{break_statement:?}").split('(').next(),
        Some("Break")
    );
    assert_eq!(break_statement.span().start.line, 4);
    assert_eq!(break_statement.span().start.column, 11);
    assert_eq!(break_statement.span().end.column, 17);
    assert_eq!(
        format!("{continue_statement:?}").split('(').next(),
        Some("Continue")
    );
    assert_eq!(continue_statement.span().start.line, 5);
    assert_eq!(continue_statement.span().start.column, 11);
    assert_eq!(continue_statement.span().end.column, 20);
}

#[test]
fn parse_should_require_semicolons_after_loop_control() {
    let result = parse_source(
        r#"
      export fn control() -> i32 {
        while true {
          break
        }
        return 0;
      }
    "#,
    );

    assert!(result.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == calckernel::DiagnosticCode::Ck1001
            && diagnostic.message == "Expected ';' after 'break'."
            && diagnostic.line == 5
            && diagnostic.column == 9
    }));
}
