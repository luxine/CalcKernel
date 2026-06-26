import { describe, expect, it } from "vitest";
import { lex } from "../src/lexer/lexer.js";
import { parse } from "../src/parser/parser.js";
import { formatDiagnostic } from "../src/source/diagnostics.js";
import { SourceFile } from "../src/source/source-file.js";
import { check } from "../src/typeck/checker.js";

function firstDiagnostic(source: SourceFile, stage: "lex" | "parse" | "check") {
  const result =
    stage === "lex"
      ? lex(source)
      : stage === "parse"
        ? parse(source)
        : check(source);
  const diagnostic = result.diagnostics[0];
  expect(diagnostic).toBeDefined();
  return diagnostic!;
}

describe("diagnostics formatter", () => {
  it("formats unknown variable diagnostics with a code and source span", () => {
    const source = new SourceFile("test.ck", "export fn bad() -> i32 {\n  return missing;\n}\n");

    expect(formatDiagnostic(source, firstDiagnostic(source, "check"))).toMatchInlineSnapshot(`
      "test.ck:2:10: error CK2001: Unknown variable 'missing'.
        return missing;
               ^^^^^^^
      "
    `);
  });

  it("formats type mismatch diagnostics with a code and source span", () => {
    const source = new SourceFile("test.ck", "export fn bad() -> i32 {\n  return true;\n}\n");

    expect(formatDiagnostic(source, firstDiagnostic(source, "check"))).toMatchInlineSnapshot(`
      "test.ck:2:10: error CK2004: Return type mismatch: expected i32 but got bool.
        return true;
               ^^^^
      "
    `);
  });

  it("formats parser diagnostics with a code and source span", () => {
    const source = new SourceFile("test.ck", "export fn bad() -> i32 {\n  let x: i32 = 1\n  return x;\n}\n");

    expect(formatDiagnostic(source, firstDiagnostic(source, "parse"))).toMatchInlineSnapshot(`
      "test.ck:3:3: error CK1001: Expected ';' after let statement.
        return x;
        ^^^^^^
      "
    `);
  });

  it("formats lexer diagnostics with a code and source span", () => {
    const source = new SourceFile("test.ck", "export fn bad() -> i32 {\n  return @;\n}\n");

    expect(formatDiagnostic(source, firstDiagnostic(source, "lex"))).toMatchInlineSnapshot(`
      "test.ck:2:10: error CK0001: Unexpected character '@'.
        return @;
               ^
      "
    `);
  });
});
