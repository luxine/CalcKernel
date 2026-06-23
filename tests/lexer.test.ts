import { describe, expect, it } from "vitest";
import { lex } from "../src/lexer/lexer.js";
import { TokenKind } from "../src/lexer/token.js";
import { SourceFile } from "../src/source/source-file.js";

function kindsOf(sourceText: string): TokenKind[] {
  const result = lex(new SourceFile("test.ik", sourceText));
  expect(result.diagnostics).toEqual([]);
  return result.tokens.map((token) => token.kind);
}

describe("lexer", () => {
  it("emits tokens with kind, text, line, column, start, and end", () => {
    const result = lex(new SourceFile("test.ik", "let x: i32 = 42;"));

    expect(result.diagnostics).toEqual([]);
    expect(result.tokens[0]).toEqual({
      kind: TokenKind.Let,
      text: "let",
      line: 1,
      column: 1,
      start: 0,
      end: 3
    });
    expect(result.tokens[5]).toMatchObject({
      kind: TokenKind.Integer,
      text: "42",
      line: 1,
      column: 14,
      start: 13,
      end: 15
    });
  });

  it("tokenizes all V0 keywords and type keywords", () => {
    expect(kindsOf("struct export fn let return if else while true false i32 i64 u32 u64 bool ptr")).toEqual([
      TokenKind.Struct,
      TokenKind.Export,
      TokenKind.Fn,
      TokenKind.Let,
      TokenKind.Return,
      TokenKind.If,
      TokenKind.Else,
      TokenKind.While,
      TokenKind.True,
      TokenKind.False,
      TokenKind.I32,
      TokenKind.I64,
      TokenKind.U32,
      TokenKind.U64,
      TokenKind.Bool,
      TokenKind.Ptr,
      TokenKind.Eof
    ]);
  });

  it("tokenizes declarations, keywords, identifiers, and punctuation", () => {
    expect(
      kindsOf("export fn add(a: i64, b: i64) -> i64 { return a + b; }")
    ).toEqual([
      TokenKind.Export,
      TokenKind.Fn,
      TokenKind.Identifier,
      TokenKind.LeftParen,
      TokenKind.Identifier,
      TokenKind.Colon,
      TokenKind.I64,
      TokenKind.Comma,
      TokenKind.Identifier,
      TokenKind.Colon,
      TokenKind.I64,
      TokenKind.RightParen,
      TokenKind.Arrow,
      TokenKind.I64,
      TokenKind.LeftBrace,
      TokenKind.Return,
      TokenKind.Identifier,
      TokenKind.Plus,
      TokenKind.Identifier,
      TokenKind.Semicolon,
      TokenKind.RightBrace,
      TokenKind.Eof
    ]);
  });

  it("tokenizes all V0 operators and delimiters", () => {
    expect(kindsOf("a==b != c <= d >= e && f || !g * h / i % m - j + k [0].x = y; ( ) { } < >")).toEqual([
      TokenKind.Identifier,
      TokenKind.EqualEqual,
      TokenKind.Identifier,
      TokenKind.BangEqual,
      TokenKind.Identifier,
      TokenKind.LessEqual,
      TokenKind.Identifier,
      TokenKind.GreaterEqual,
      TokenKind.Identifier,
      TokenKind.AmpAmp,
      TokenKind.Identifier,
      TokenKind.PipePipe,
      TokenKind.Bang,
      TokenKind.Identifier,
      TokenKind.Star,
      TokenKind.Identifier,
      TokenKind.Slash,
      TokenKind.Identifier,
      TokenKind.Percent,
      TokenKind.Identifier,
      TokenKind.Minus,
      TokenKind.Identifier,
      TokenKind.Plus,
      TokenKind.Identifier,
      TokenKind.LeftBracket,
      TokenKind.Integer,
      TokenKind.RightBracket,
      TokenKind.Dot,
      TokenKind.Identifier,
      TokenKind.Equal,
      TokenKind.Identifier,
      TokenKind.Semicolon,
      TokenKind.LeftParen,
      TokenKind.RightParen,
      TokenKind.LeftBrace,
      TokenKind.RightBrace,
      TokenKind.Less,
      TokenKind.Greater,
      TokenKind.Eof
    ]);
  });

  it("tracks line and column for tokens", () => {
    const result = lex(new SourceFile("test.ik", "let x: i32 = 1;\n  return x;"));

    expect(result.diagnostics).toEqual([]);
    expect(result.tokens.find((token) => token.text === "return")).toMatchObject({
      start: 18,
      end: 24,
      line: 2,
      column: 3
    });
  });

  it("skips whitespace and line comments", () => {
    expect(kindsOf("let x: i32 = 1; // ignored\nreturn x;")).toEqual([
      TokenKind.Let,
      TokenKind.Identifier,
      TokenKind.Colon,
      TokenKind.I32,
      TokenKind.Equal,
      TokenKind.Integer,
      TokenKind.Semicolon,
      TokenKind.Return,
      TokenKind.Identifier,
      TokenKind.Semicolon,
      TokenKind.Eof
    ]);
  });

  it("reports unknown characters with line and column", () => {
    const result = lex(new SourceFile("bad.ik", "let x: i32 = @;"));

    expect(result.diagnostics).toEqual([
      expect.objectContaining({
        code: "IK0001",
        severity: "error",
        message: "Unexpected character '@'.",
        fileName: "bad.ik",
        line: 1,
        column: 14
      })
    ]);
    expect(result.diagnostics[0]?.span).toMatchObject({
      start: { line: 1, column: 14, offset: 13 },
      end: { line: 1, column: 15, offset: 14 }
    });
  });
});
