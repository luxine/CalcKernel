import { describe, expect, it, vi } from "vitest";

const vscodeMockState = vi.hoisted(() => ({
  registeredProvider: undefined as
    | {
        provideDocumentSemanticTokens(document: unknown): { data: number[] };
      }
    | undefined
}));

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

  class SemanticTokensLegend {
    constructor(
      readonly tokenTypes: string[],
      readonly tokenModifiers: string[]
    ) {}
  }

  class SemanticTokensBuilder {
    private readonly data: number[] = [];

    constructor(readonly legend: SemanticTokensLegend) {}

    push(line: number, character: number, length: number, tokenType: number, tokenModifiers: number): void {
      this.data.push(line, character, length, tokenType, tokenModifiers);
    }

    build(): { data: number[] } {
      return { data: this.data };
    }
  }

  return {
    Diagnostic,
    DiagnosticSeverity: { Error: 0 },
    languages: {
      registerDocumentSemanticTokensProvider: (_selector: unknown, provider: typeof vscodeMockState.registeredProvider) => {
        vscodeMockState.registeredProvider = provider;
        return { dispose: vi.fn() };
      }
    },
    Position,
    Range,
    SemanticTokensBuilder,
    SemanticTokensLegend,
    Uri: {
      parse: (uri: string) => ({
        toString: () => uri
      })
    }
  };
});

import * as vscode from "vscode";
import { analyzeIntKernelDocument, createMemoryDocument } from "../src/languageService";
import { buildSemanticTokenData, registerSemanticTokens, semanticTokenLegend, type SemanticTokenData } from "../src/semanticTokens";

const sourceText = `
struct Item {
  price: i64;
}

fn tax() -> i64 {
  return 1;
}

fn total(item: Item) -> i64 {
  return item.price + tax();
}
`.trimStart();

describe("semanticTokens", () => {
  it("classifies declarations and references", () => {
    const analysis = analyzeIntKernelDocument(createMemoryDocument(sourceText));
    const tokens = buildSemanticTokenData(analysis);
    const typeIndex = semanticTokenLegend.tokenTypes.indexOf("type");
    const functionIndex = semanticTokenLegend.tokenTypes.indexOf("function");
    const parameterIndex = semanticTokenLegend.tokenTypes.indexOf("parameter");
    const propertyIndex = semanticTokenLegend.tokenTypes.indexOf("property");
    const declarationModifier = 1 << semanticTokenLegend.tokenModifiers.indexOf("declaration");

    expect(tokenAt(tokens, "Item", 0, typeIndex)?.tokenModifiers).toBe(declarationModifier);
    expect(tokenAt(tokens, "Item", 8, typeIndex)?.tokenModifiers).toBe(0);
    expect(tokenAt(tokens, "price", 1, propertyIndex)?.tokenModifiers).toBe(declarationModifier);
    expect(tokenAt(tokens, "price", 9, propertyIndex)?.tokenModifiers).toBe(0);
    expect(tokenAt(tokens, "item", 8, parameterIndex)?.tokenModifiers).toBe(declarationModifier);
    expect(tokenAt(tokens, "item", 9, parameterIndex)?.tokenModifiers).toBe(0);
    expect(tokenAt(tokens, "tax", 4, functionIndex)?.tokenModifiers).toBe(declarationModifier);
    expect(tokenAt(tokens, "tax", 9, functionIndex)?.tokenModifiers).toBe(0);
  });

  it("registers a provider that returns semantic token data for a document", () => {
    const context = { subscriptions: [] as Array<{ dispose(): void }> };
    registerSemanticTokens(context as vscode.ExtensionContext);
    const document = createMemoryDocument(sourceText);
    const result = vscodeMockState.registeredProvider?.provideDocumentSemanticTokens(document);

    expect(context.subscriptions).toHaveLength(1);
    expect(result?.data.length).toBeGreaterThan(0);
  });
});

function tokenAt(tokens: SemanticTokenData[], text: string, line: number, tokenType: number): SemanticTokenData | undefined {
  return tokens.find((token) => token.text === text && token.range.start.line === line && token.tokenType === tokenType);
}
