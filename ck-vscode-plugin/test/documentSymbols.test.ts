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

  class DocumentSymbol {
    children: DocumentSymbol[] = [];

    constructor(
      readonly name: string,
      readonly detail: string,
      readonly kind: number,
      readonly range: Range,
      readonly selectionRange: Range
    ) {}
  }

  return {
    Diagnostic,
    DiagnosticSeverity: { Error: 0 },
    DocumentSymbol,
    languages: {
      registerDocumentSymbolProvider: vi.fn()
    },
    Position,
    Range,
    SymbolKind: {
      Struct: 22,
      Field: 7,
      Function: 11,
      Variable: 12
    },
    Uri: {
      parse: (uri: string) => ({
        toString: () => uri
      })
    }
  };
});

import { buildDocumentSymbols } from "../src/documentSymbols";
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

describe("documentSymbols", () => {
  it("builds outline entries for structs, fields, functions, params, and locals", () => {
    const analysis = analyzeCalcKernelDocument(createMemoryDocument(sourceText));
    const symbols = buildDocumentSymbols(analysis);
    expect(symbols.map((symbol) => symbol.name)).toEqual(["Item", "total"]);
    expect(symbols[0]?.children.map((symbol) => symbol.name)).toEqual(["price"]);
    expect(symbols[1]?.children.map((symbol) => symbol.name)).toEqual(["item", "subtotal"]);
  });

  it("uses parent ranges that contain child symbol selections", () => {
    const analysis = analyzeCalcKernelDocument(createMemoryDocument(sourceText, "memory:///ranges.ck"));
    const symbols = buildDocumentSymbols(analysis);
    const item = symbols[0];
    const price = item?.children[0];
    const total = symbols[1];
    const itemParam = total?.children[0];
    const subtotal = total?.children[1];

    expect(item?.range.contains(price!.selectionRange.start)).toBe(true);
    expect(item?.range.contains(price!.selectionRange.end)).toBe(true);
    expect(total?.range.contains(itemParam!.selectionRange.start)).toBe(true);
    expect(total?.range.contains(itemParam!.selectionRange.end)).toBe(true);
    expect(total?.range.contains(subtotal!.selectionRange.start)).toBe(true);
    expect(total?.range.contains(subtotal!.selectionRange.end)).toBe(true);
  });

  it("attaches params and locals only to their owning function", () => {
    const twoFunctionSourceText = `
fn first(value: i64) -> i64 {
  let total: i64 = value;
  return total;
}

fn second(item: i64) -> i64 {
  let subtotal: i64 = item;
  return subtotal;
}
`.trimStart();
    const analysis = analyzeCalcKernelDocument(createMemoryDocument(twoFunctionSourceText, "memory:///two-functions.ck"));
    const symbols = buildDocumentSymbols(analysis);

    expect(symbols.map((symbol) => symbol.name)).toEqual(["first", "second"]);
    expect(symbols[0]?.children.map((symbol) => symbol.name)).toEqual(["value", "total"]);
    expect(symbols[1]?.children.map((symbol) => symbol.name)).toEqual(["item", "subtotal"]);
  });
});
