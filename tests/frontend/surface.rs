use calckernel::{
    CheckResult, Expression, ParseResult, Program, SourceFile, TokenKind, check, lex, parse,
};

#[test]
fn frontend_types_and_functions_should_remain_flat_root_exports() {
    let source = SourceFile::new(
        "surface.ck",
        "export fn identity(value: i32) -> i32 { return value; }",
    );
    let lexed = lex(&source);
    assert_eq!(
        lexed.tokens.last().map(|token| token.kind),
        Some(TokenKind::Eof)
    );

    let parsed: ParseResult = parse(&source);
    assert!(parsed.diagnostics.is_empty());
    let _: &Program = &parsed.ast;

    let checked: CheckResult = check(&source);
    assert!(checked.diagnostics.is_empty());
    let _: Option<Expression> = None;
}
