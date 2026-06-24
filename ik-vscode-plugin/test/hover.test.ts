import { describe, expect, it, vi } from "vitest";

vi.mock("vscode", () => {
  class Position {
    constructor(
      readonly line: number,
      readonly character: number
    ) {}

    compareTo(other: Position): number {
      if (this.line !== other.line) {
        return this.line - other.line;
      }
      return this.character - other.character;
    }
  }

  class Range {
    readonly start: Position;
    readonly end: Position;

    constructor(startLine: number, startCharacter: number, endLine: number, endCharacter: number) {
      this.start = new Position(startLine, startCharacter);
      this.end = new Position(endLine, endCharacter);
    }

    contains(position: Position): boolean {
      if (position.line < this.start.line || position.line > this.end.line) {
        return false;
      }
      if (position.line === this.start.line && position.character < this.start.character) {
        return false;
      }
      if (position.line === this.end.line && position.character > this.end.character) {
        return false;
      }
      return true;
    }
  }

  class Diagnostic {
    code?: string;
    source?: string;

    constructor(
      readonly range: Range,
      readonly message: string,
      readonly severity: number
    ) {}
  }

  class MarkdownString {
    constructor(readonly value: string) {}
  }

  return {
    Diagnostic,
    DiagnosticSeverity: { Error: 0 },
    languages: {
      registerHoverProvider: vi.fn()
    },
    MarkdownString,
    Position,
    Range,
    Uri: {
      parse: (uri: string) => ({
        toString: () => uri
      })
    }
  };
});

import * as vscode from "vscode";
import { getHoverMarkdownAtPosition } from "../src/hover";
import { analyzeIntKernelDocument, createMemoryDocument } from "../src/languageService";

const sourceText = `
struct Item {
  price: i64;
}

fn total(item: Item) -> i64 {
  let subtotal: i64 = item.price;
  return subtotal;
}
`.trimStart();

describe("hover", () => {
  it("shows local variable type information", () => {
    const analysis = analyzeIntKernelDocument(createMemoryDocument(sourceText));
    const markdown = getHoverMarkdownAtPosition(analysis, new vscode.Position(6, 9));
    expect(markdown?.value).toContain("local subtotal: i64");
  });

  it("shows function signature information", () => {
    const analysis = analyzeIntKernelDocument(createMemoryDocument(sourceText));
    const markdown = getHoverMarkdownAtPosition(analysis, new vscode.Position(4, 3));
    expect(markdown?.value).toContain("fn total(item: Item) -> i64");
  });

  it("shows field type information", () => {
    const analysis = analyzeIntKernelDocument(createMemoryDocument(sourceText));
    const markdown = getHoverMarkdownAtPosition(analysis, new vscode.Position(5, 28));
    expect(markdown?.value).toContain("field Item.price: i64");
  });
});
