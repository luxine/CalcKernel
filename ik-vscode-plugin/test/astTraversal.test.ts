import { SourceFile, parse } from "calckernel";
import { describe, expect, it } from "vitest";
import { containsPosition, walkProgram } from "../src/astTraversal";

const sourceText = `
struct Item {
  price: i64;
}

fn add_tax(price: i64, tax: i64) -> i64 {
  let total: i64 = price + tax;
  return total;
}
`.trimStart();

describe("astTraversal", () => {
  it("visits declarations, statements, and expressions in source order", () => {
    const parsed = parse(new SourceFile("sample.ck", sourceText));
    const visits: string[] = [];

    walkProgram(parsed.ast, {
      declaration: (node) => visits.push(node.kind),
      statement: (node) => visits.push(node.kind),
      expression: (node) => visits.push(node.kind)
    });

    expect(visits).toEqual([
      "StructDeclaration",
      "FunctionDeclaration",
      "LetStatement",
      "BinaryExpression",
      "IdentifierExpression",
      "IdentifierExpression",
      "ReturnStatement",
      "IdentifierExpression"
    ]);
  });

  it("checks zero-based positions against one-based compiler spans", () => {
    const parsed = parse(new SourceFile("sample.ck", sourceText));
    const functionDeclaration = parsed.ast.declarations[1]!;
    const endPosition = {
      line: functionDeclaration.span.end.line - 1,
      character: functionDeclaration.span.end.column - 1
    };

    expect(containsPosition(functionDeclaration.span, { line: 5, character: 3 })).toBe(true);
    expect(containsPosition(functionDeclaration.span, endPosition)).toBe(false);
    expect(containsPosition(functionDeclaration.span, { line: 0, character: 0 })).toBe(false);
  });
});
