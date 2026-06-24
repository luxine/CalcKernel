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

  return {
    Diagnostic,
    DiagnosticSeverity: { Error: 0 },
    Position,
    Range,
    Uri: {
      parse: (uri: string) => ({
        toString: () => uri
      })
    }
  };
});

import { analyzeIntKernelDocument, createMemoryDocument } from "../src/languageService";

const sourceText = `
struct Item {
  price: i64;
  qty: i64;
}

fn line_total(item: Item, tax_rate: i64) -> i64 {
  let subtotal: i64 = item.price * item.qty;
  return subtotal + tax_rate;
}
`.trimStart();

describe("languageService", () => {
  it("extracts document-local symbols and references", () => {
    const document = createMemoryDocument(sourceText);
    const analysis = analyzeIntKernelDocument(document);

    expect(analysis.diagnostics).toHaveLength(0);
    expect(analysis.symbols.map((symbol) => `${symbol.kind}:${symbol.name}`)).toEqual([
      "struct:Item",
      "field:price",
      "field:qty",
      "function:line_total",
      "parameter:item",
      "parameter:tax_rate",
      "local:subtotal"
    ]);
    expect(analysis.references.some((reference) => reference.kind === "field" && reference.name === "price")).toBe(true);
    expect(analysis.references.some((reference) => reference.kind === "local" && reference.name === "subtotal")).toBe(true);
  });

  it("reuses cached analysis for the same URI and version", () => {
    const document = createMemoryDocument(sourceText, "memory:///sample.ik", 7);
    const first = analyzeIntKernelDocument(document);
    const second = analyzeIntKernelDocument(document);
    expect(second).toBe(first);
  });

  it("evicts older cached versions for the same URI", () => {
    const uri = "memory:///versioned.ik";
    const versionOne = createMemoryDocument(sourceText, uri, 1);
    const versionTwo = createMemoryDocument(sourceText, uri, 2);
    const firstVersionOne = analyzeIntKernelDocument(versionOne);
    const firstVersionTwo = analyzeIntKernelDocument(versionTwo);
    const secondVersionOne = analyzeIntKernelDocument(versionOne);

    expect(firstVersionTwo).not.toBe(firstVersionOne);
    expect(secondVersionOne).not.toBe(firstVersionOne);
    expect(secondVersionOne.document.version).toBe(1);
    expect(secondVersionOne.diagnostics).toHaveLength(0);
  });

  it("links identifier references to same-function symbols when names repeat", () => {
    const scopedSourceText = `
fn first(value: i64) -> i64 {
  let total: i64 = value;
  return total;
}

fn second(value: i64) -> i64 {
  let total: i64 = value;
  return total + value;
}
`.trimStart();
    const document = createMemoryDocument(scopedSourceText, "memory:///scope.ik", 1);
    const analysis = analyzeIntKernelDocument(document);

    expect(analysis.diagnostics).toHaveLength(0);
    const secondValueSymbol = analysis.symbols.find(
      (symbol) => symbol.kind === "parameter" && symbol.name === "value" && symbol.selectionRange.start.line === 5
    );
    const secondTotalSymbol = analysis.symbols.find(
      (symbol) => symbol.kind === "local" && symbol.name === "total" && symbol.selectionRange.start.line === 6
    );
    const secondValueReference = analysis.references.find(
      (reference) => reference.kind === "parameter" && reference.name === "value" && reference.range.start.line === 7
    );
    const secondTotalReference = analysis.references.find(
      (reference) => reference.kind === "local" && reference.name === "total" && reference.range.start.line === 7
    );

    expect(secondValueReference?.target).toBe(secondValueSymbol);
    expect(secondTotalReference?.target).toBe(secondTotalSymbol);
  });

  it("does not let injected check failures poison normal cached analysis", () => {
    const document = createMemoryDocument(sourceText, "memory:///injected-cache.ik", 1);
    const injected = analyzeIntKernelDocument(document, {
      checkDocument: () => {
        throw new Error("forced failure");
      }
    });
    const normal = analyzeIntKernelDocument(document);

    expect(injected.diagnostics).toHaveLength(1);
    expect(injected.diagnostics[0]?.message).toContain("IntKernel validation failed: forced failure");
    expect(normal.diagnostics).toHaveLength(0);
    expect(normal.symbols.map((symbol) => `${symbol.kind}:${symbol.name}`)).toContain("function:line_total");
    expect(normal).not.toBe(injected);
  });

  it("prefers scoped identifiers over same-name functions", () => {
    const shadowedFunctionSourceText = `
fn value() -> i64 {
  return 1;
}

fn use_value(value: i64) -> i64 {
  let other: i64 = value;
  return other + value;
}
`.trimStart();
    const document = createMemoryDocument(shadowedFunctionSourceText, "memory:///function-shadow.ik", 1);
    const analysis = analyzeIntKernelDocument(document);

    expect(analysis.diagnostics).toHaveLength(0);
    const valueFunctionSymbol = analysis.symbols.find((symbol) => symbol.kind === "function" && symbol.name === "value");
    const parameterSymbol = analysis.symbols.find(
      (symbol) => symbol.kind === "parameter" && symbol.name === "value" && symbol.selectionRange.start.line === 4
    );
    const firstParameterReference = analysis.references.find(
      (reference) => reference.name === "value" && reference.range.start.line === 5
    );
    const secondParameterReference = analysis.references.find(
      (reference) => reference.name === "value" && reference.range.start.line === 6
    );

    expect(firstParameterReference?.kind).toBe("parameter");
    expect(firstParameterReference?.target).toBe(parameterSymbol);
    expect(firstParameterReference?.target).not.toBe(valueFunctionSymbol);
    expect(secondParameterReference?.kind).toBe("parameter");
    expect(secondParameterReference?.target).toBe(parameterSymbol);
  });

  it("prefers function symbols for call callee identifiers", () => {
    const callSourceText = `
fn value() -> i64 {
  return 1;
}

fn use_value(value: i64) -> i64 {
  return value();
}
`.trimStart();
    const document = createMemoryDocument(callSourceText, "memory:///call-shadow.ik", 1);
    const analysis = analyzeIntKernelDocument(document);

    expect(analysis.diagnostics).toHaveLength(0);
    const valueFunctionSymbol = analysis.symbols.find((symbol) => symbol.kind === "function" && symbol.name === "value");
    const valueCallReference = analysis.references.find(
      (reference) => reference.name === "value" && reference.range.start.line === 5
    );

    expect(valueCallReference?.kind).toBe("function");
    expect(valueCallReference?.target).toBe(valueFunctionSymbol);
  });

  it("indexes named type references in annotations", () => {
    const typedSourceText = `
struct Item {
  next: ptr<Item>;
}

fn clone_item(item: Item) -> Item {
  let copy: Item = item;
  return copy;
}
`.trimStart();
    const document = createMemoryDocument(typedSourceText, "memory:///types.ik", 1);
    const analysis = analyzeIntKernelDocument(document);

    expect(analysis.diagnostics).toHaveLength(0);
    const itemStructSymbol = analysis.symbols.find((symbol) => symbol.kind === "struct" && symbol.name === "Item");
    const itemTypeReferences = analysis.references.filter((reference) => reference.kind === "type" && reference.name === "Item");

    expect(itemTypeReferences).toHaveLength(4);
    expect(itemTypeReferences.map((reference) => reference.range.start.line)).toEqual([1, 4, 4, 5]);
    expect(itemTypeReferences.every((reference) => reference.target === itemStructSymbol)).toBe(true);
  });

  it("falls back to a single diagnostic when check throws", () => {
    const document = createMemoryDocument(sourceText, "memory:///sample.ik", 1);
    const analysis = analyzeIntKernelDocument(document, {
      checkDocument: () => {
        throw new Error("forced failure");
      }
    });

    expect(analysis.diagnostics).toHaveLength(1);
    expect(analysis.diagnostics[0]?.message).toContain("IntKernel validation failed: forced failure");
    expect(analysis.symbols).toHaveLength(0);
    expect(analysis.references).toHaveLength(0);
  });

  it("converts compiler diagnostics to vscode diagnostics", () => {
    const invalid = `
fn broken() -> i64 {
  let value: i64 = true;
  return value;
}
`.trimStart();
    const analysis = analyzeIntKernelDocument(createMemoryDocument(invalid, "memory:///broken.ik", 1));
    expect(analysis.diagnostics.length).toBeGreaterThan(0);
    expect(analysis.diagnostics[0]?.source).toBe("intkernel");
    expect(analysis.diagnostics.some((diagnostic) => diagnostic.message.includes("Cannot initialize"))).toBe(true);
  });
});
