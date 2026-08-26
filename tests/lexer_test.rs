use calckernel::{
    Diagnostic, DiagnosticCode, SourceFile, SourcePosition, SourceSpan, TokenKind,
    format_diagnostic, lex,
};

#[test]
fn lex_should_emit_eof_for_empty_source() {
    let source = SourceFile::new("empty.ck", "");
    let result = lex(&source);

    assert_eq!(result.tokens.len(), 1);
    assert_eq!(result.tokens[0].kind, TokenKind::Eof);
}

#[test]
fn format_diagnostic_should_match_typescript_caret_output() {
    let source = SourceFile::new("test.ck", "export fn bad() -> i32 {\n  return @;\n}\n");
    let diagnostic = Diagnostic::error(
        DiagnosticCode::Ck0001,
        "Unexpected character '@'.",
        "test.ck",
        SourceSpan {
            start: SourcePosition {
                offset: 34,
                line: 2,
                column: 10,
            },
            end: SourcePosition {
                offset: 35,
                line: 2,
                column: 11,
            },
        },
    );

    assert_eq!(
        format_diagnostic(&source, &diagnostic),
        "test.ck:2:10: error CK0001: Unexpected character '@'.\n  return @;\n         ^\n"
    );
}

#[test]
fn lex_should_emit_keywords_identifiers_numbers_and_punctuation() {
    let source = SourceFile::new(
        "test.ck",
        "export fn add(a: i64, b: i64) -> i64 { return a + b; }\n",
    );
    let result = lex(&source);
    let kinds: Vec<TokenKind> = result.tokens.iter().map(|token| token.kind).collect();

    assert_eq!(
        kinds,
        vec![
            TokenKind::Export,
            TokenKind::Fn,
            TokenKind::Identifier,
            TokenKind::LeftParen,
            TokenKind::Identifier,
            TokenKind::Colon,
            TokenKind::I64,
            TokenKind::Comma,
            TokenKind::Identifier,
            TokenKind::Colon,
            TokenKind::I64,
            TokenKind::RightParen,
            TokenKind::Arrow,
            TokenKind::I64,
            TokenKind::LeftBrace,
            TokenKind::Return,
            TokenKind::Identifier,
            TokenKind::Plus,
            TokenKind::Identifier,
            TokenKind::Semicolon,
            TokenKind::RightBrace,
            TokenKind::Eof,
        ]
    );
    assert_eq!(result.tokens[0].text, "export");
    assert_eq!(result.tokens[0].line, 1);
    assert_eq!(result.tokens[0].column, 1);
}

#[test]
fn lex_should_emit_token_metadata_for_let_statement() {
    let source = SourceFile::new("test.ck", "let x: i32 = 42;");
    let result = lex(&source);

    assert_eq!(result.diagnostics, []);
    assert_eq!(result.tokens[0].kind, TokenKind::Let);
    assert_eq!(result.tokens[0].text, "let");
    assert_eq!(result.tokens[0].line, 1);
    assert_eq!(result.tokens[0].column, 1);
    assert_eq!(result.tokens[0].start, 0);
    assert_eq!(result.tokens[0].end, 3);
    assert_eq!(result.tokens[5].kind, TokenKind::Integer);
    assert_eq!(result.tokens[5].text, "42");
    assert_eq!(result.tokens[5].line, 1);
    assert_eq!(result.tokens[5].column, 14);
    assert_eq!(result.tokens[5].start, 13);
    assert_eq!(result.tokens[5].end, 15);
}

#[test]
fn lex_should_expose_typescript_utf16_offsets_after_non_bmp_characters() {
    let source = SourceFile::new("test.ck", "🙂 let x: i32 = 1;");
    let result = lex(&source);

    assert_eq!(result.diagnostics.len(), 2);
    assert_eq!(result.diagnostics[0].span.start.offset, 0);
    assert_eq!(result.diagnostics[0].span.end.offset, 1);
    assert_eq!(result.diagnostics[1].span.start.offset, 1);
    assert_eq!(result.diagnostics[1].span.end.offset, 2);

    assert_eq!(result.tokens[0].kind, TokenKind::Let);
    assert_eq!(result.tokens[0].text, "let");
    assert_eq!(result.tokens[0].line, 1);
    assert_eq!(result.tokens[0].column, 4);
    assert_eq!(result.tokens[0].start, 3);
    assert_eq!(result.tokens[0].end, 6);
    assert_eq!(result.tokens[1].text, "x");
    assert_eq!(result.tokens[1].start, 7);
    assert_eq!(result.tokens[1].end, 8);
}

#[test]
fn lex_should_tokenize_all_v0_keywords_and_type_keywords() {
    let source = SourceFile::new(
        "test.ck",
        "struct export fn let return if else while true false i32 i64 u32 u64 f64 bool ptr",
    );
    let result = lex(&source);
    let kinds: Vec<TokenKind> = result.tokens.iter().map(|token| token.kind).collect();

    assert_eq!(result.diagnostics, []);
    assert_eq!(
        kinds,
        vec![
            TokenKind::Struct,
            TokenKind::Export,
            TokenKind::Fn,
            TokenKind::Let,
            TokenKind::Return,
            TokenKind::If,
            TokenKind::Else,
            TokenKind::While,
            TokenKind::True,
            TokenKind::False,
            TokenKind::I32,
            TokenKind::I64,
            TokenKind::U32,
            TokenKind::U64,
            TokenKind::F64,
            TokenKind::Bool,
            TokenKind::Ptr,
            TokenKind::Eof,
        ]
    );
}

#[test]
fn lex_should_tokenize_all_v0_operators_and_delimiters() {
    let source = SourceFile::new(
        "test.ck",
        "a==b != c <= d >= e && f || !g * h / i % m - j + k [0].x = y; ( ) { } < >",
    );
    let result = lex(&source);
    let kinds: Vec<TokenKind> = result.tokens.iter().map(|token| token.kind).collect();

    assert_eq!(result.diagnostics, []);
    assert_eq!(
        kinds,
        vec![
            TokenKind::Identifier,
            TokenKind::EqualEqual,
            TokenKind::Identifier,
            TokenKind::BangEqual,
            TokenKind::Identifier,
            TokenKind::LessEqual,
            TokenKind::Identifier,
            TokenKind::GreaterEqual,
            TokenKind::Identifier,
            TokenKind::AmpAmp,
            TokenKind::Identifier,
            TokenKind::PipePipe,
            TokenKind::Bang,
            TokenKind::Identifier,
            TokenKind::Star,
            TokenKind::Identifier,
            TokenKind::Slash,
            TokenKind::Identifier,
            TokenKind::Percent,
            TokenKind::Identifier,
            TokenKind::Minus,
            TokenKind::Identifier,
            TokenKind::Plus,
            TokenKind::Identifier,
            TokenKind::LeftBracket,
            TokenKind::Integer,
            TokenKind::RightBracket,
            TokenKind::Dot,
            TokenKind::Identifier,
            TokenKind::Equal,
            TokenKind::Identifier,
            TokenKind::Semicolon,
            TokenKind::LeftParen,
            TokenKind::RightParen,
            TokenKind::LeftBrace,
            TokenKind::RightBrace,
            TokenKind::Less,
            TokenKind::Greater,
            TokenKind::Eof,
        ]
    );
}

#[test]
fn lex_should_tokenize_supported_float_literals_as_single_tokens() {
    let source = SourceFile::new("test.ck", "1.0 0.5 1e3 1.0e-3 2E8 2E+8");
    let result = lex(&source);
    let actual: Vec<(TokenKind, &str)> = result
        .tokens
        .iter()
        .map(|token| (token.kind, token.text.as_str()))
        .collect();

    assert_eq!(result.diagnostics, []);
    assert_eq!(
        actual,
        vec![
            (TokenKind::Float, "1.0"),
            (TokenKind::Float, "0.5"),
            (TokenKind::Float, "1e3"),
            (TokenKind::Float, "1.0e-3"),
            (TokenKind::Float, "2E8"),
            (TokenKind::Float, "2E+8"),
            (TokenKind::Eof, ""),
        ]
    );
}

#[test]
fn lex_should_report_malformed_float_literals() {
    for (input, expected) in [
        ("1.", "Malformed float literal '1.'."),
        (".5", "Malformed float literal '.5'."),
        ("1e", "Malformed float literal '1e'."),
        ("1e+", "Malformed float literal '1e+'."),
    ] {
        let source = SourceFile::new("test.ck", input);
        let result = lex(&source);

        assert_eq!(result.diagnostics.len(), 1, "input: {input}");
        assert_eq!(result.diagnostics[0].code, DiagnosticCode::Ck0001);
        assert_eq!(result.diagnostics[0].message, expected);
    }
}

#[test]
fn lex_should_track_line_and_column_for_tokens() {
    let source = SourceFile::new("test.ck", "let x: i32 = 1;\n  return x;");
    let result = lex(&source);
    let return_token = result
        .tokens
        .iter()
        .find(|token| token.text == "return")
        .expect("return token should be present");

    assert_eq!(result.diagnostics, []);
    assert_eq!(return_token.start, 18);
    assert_eq!(return_token.end, 24);
    assert_eq!(return_token.line, 2);
    assert_eq!(return_token.column, 3);
}

#[test]
fn lex_should_skip_whitespace_and_line_comments() {
    let source = SourceFile::new("test.ck", "let x: i32 = 1; // ignored\nreturn x;");
    let result = lex(&source);
    let kinds: Vec<TokenKind> = result.tokens.iter().map(|token| token.kind).collect();

    assert_eq!(result.diagnostics, []);
    assert_eq!(
        kinds,
        vec![
            TokenKind::Let,
            TokenKind::Identifier,
            TokenKind::Colon,
            TokenKind::I32,
            TokenKind::Equal,
            TokenKind::Integer,
            TokenKind::Semicolon,
            TokenKind::Return,
            TokenKind::Identifier,
            TokenKind::Semicolon,
            TokenKind::Eof,
        ]
    );
}

#[test]
fn lex_should_report_unknown_characters_with_line_and_column() {
    let source = SourceFile::new("bad.ck", "let x: i32 = @;");
    let result = lex(&source);

    assert_eq!(result.diagnostics.len(), 1);
    assert_eq!(result.diagnostics[0].code, DiagnosticCode::Ck0001);
    assert_eq!(result.diagnostics[0].message, "Unexpected character '@'.");
    assert_eq!(result.diagnostics[0].file_name, "bad.ck");
    assert_eq!(result.diagnostics[0].line, 1);
    assert_eq!(result.diagnostics[0].column, 14);
    assert_eq!(
        result.diagnostics[0].span,
        SourceSpan {
            start: SourcePosition {
                line: 1,
                column: 14,
                offset: 13,
            },
            end: SourcePosition {
                line: 1,
                column: 15,
                offset: 14,
            },
        }
    );
}

#[test]
fn lex_should_tokenize_break_and_continue_as_reserved_keywords() {
    let source = SourceFile::new("test.ck", "break;\n  continue;");
    let result = lex(&source);
    let tokens = result
        .tokens
        .iter()
        .map(|token| {
            (
                format!("{:?}", token.kind),
                token.text.as_str(),
                token.line,
                token.column,
                token.start,
                token.end,
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(
        tokens,
        vec![
            ("Break".to_string(), "break", 1, 1, 0, 5),
            ("Semicolon".to_string(), ";", 1, 6, 5, 6),
            ("Continue".to_string(), "continue", 2, 3, 9, 17),
            ("Semicolon".to_string(), ";", 2, 11, 17, 18),
            ("Eof".to_string(), "", 2, 12, 18, 18),
        ]
    );
}

#[test]
fn lex_should_tokenize_void_as_a_reserved_keyword() {
    let result = lex(&SourceFile::new("test.ck", "void value"));

    assert_eq!(result.diagnostics, []);
    assert_eq!(result.tokens[0].kind, TokenKind::Void);
    assert_eq!(result.tokens[0].text, "void");
    assert_eq!(result.tokens[1].kind, TokenKind::Identifier);
}

#[test]
fn lex_should_tokenize_slice_and_dotdot_with_exact_spans() {
    let result = lex(&SourceFile::new("test.ck", "slice values[0..len]"));
    let tokens = result
        .tokens
        .iter()
        .map(|token| (token.kind, token.text.as_str(), token.start, token.end))
        .collect::<Vec<_>>();

    assert_eq!(result.diagnostics, []);
    assert_eq!(
        tokens,
        vec![
            (TokenKind::Slice, "slice", 0, 5),
            (TokenKind::Identifier, "values", 6, 12),
            (TokenKind::LeftBracket, "[", 12, 13),
            (TokenKind::Integer, "0", 13, 14),
            (TokenKind::DotDot, "..", 14, 16),
            (TokenKind::Identifier, "len", 16, 19),
            (TokenKind::RightBracket, "]", 19, 20),
            (TokenKind::Eof, "", 20, 20),
        ]
    );
}

#[test]
fn lex_should_prefer_dotdot_over_float_fraction_after_integer() {
    let result = lex(&SourceFile::new("test.ck", "1..2 1.25 3..len"));
    let actual = result
        .tokens
        .iter()
        .map(|token| (token.kind, token.text.as_str()))
        .collect::<Vec<_>>();

    assert_eq!(result.diagnostics, []);
    assert_eq!(
        actual,
        vec![
            (TokenKind::Integer, "1"),
            (TokenKind::DotDot, ".."),
            (TokenKind::Integer, "2"),
            (TokenKind::Float, "1.25"),
            (TokenKind::Integer, "3"),
            (TokenKind::DotDot, ".."),
            (TokenKind::Identifier, "len"),
            (TokenKind::Eof, ""),
        ]
    );
}

#[test]
fn lex_should_preserve_supported_and_malformed_float_behavior() {
    let valid = lex(&SourceFile::new("valid.ck", "1.0 0.5 1e3 1.0e-3"));
    assert_eq!(valid.diagnostics, []);
    assert!(
        valid
            .tokens
            .iter()
            .take(4)
            .all(|token| token.kind == TokenKind::Float)
    );

    for (text, message) in [
        ("1.", "Malformed float literal '1.'."),
        (".5", "Malformed float literal '.5'."),
    ] {
        let result = lex(&SourceFile::new("bad.ck", text));
        assert_eq!(result.diagnostics.len(), 1, "{text}");
        assert_eq!(result.diagnostics[0].message, message, "{text}");
    }
}
