import { describe, expect, it, vi } from "vitest";

vi.mock("vscode", () => {
  class Position {
    constructor(
      readonly line: number,
      readonly character: number
    ) {}
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

  class Location {
    constructor(
      readonly uri: unknown,
      readonly range: Range
    ) {}
  }

  return {
    Diagnostic,
    DiagnosticSeverity: { Error: 0 },
    languages: {
      registerDefinitionProvider: vi.fn()
    },
    Location,
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
import { getDefinitionAtPosition } from "../src/definitions";
import { analyzeCalcKernelDocument, createMemoryDocument } from "../src/languageService";

const sourceText = `
struct Item {
  price: i64;
}

fn total(item: Item) -> i64 {
  let subtotal: i64 = item.price;
  return subtotal;
}
`.trimStart();

describe("definitions", () => {
  it("resolves local variable references", () => {
    const analysis = analyzeCalcKernelDocument(createMemoryDocument(sourceText));
    const location = getDefinitionAtPosition(analysis, new vscode.Position(6, 9));
    expect(location?.range.start.line).toBe(5);
    expect(location?.range.start.character).toBe(6);
    expect(location?.range.end.character).toBe(14);
  });

  it("resolves field references", () => {
    const analysis = analyzeCalcKernelDocument(createMemoryDocument(sourceText));
    const location = getDefinitionAtPosition(analysis, new vscode.Position(5, 28));
    expect(location?.range.start.line).toBe(1);
    expect(location?.range.start.character).toBe(2);
    expect(location?.range.end.character).toBe(7);
  });
});
