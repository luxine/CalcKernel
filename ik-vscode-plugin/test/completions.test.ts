import { describe, expect, it, vi } from "vitest";

const vscodeMockState = vi.hoisted(() => ({
  registeredProvider: undefined as
    | {
        provideCompletionItems(document: unknown, position: unknown): unknown;
      }
    | undefined,
  triggerCharacters: [] as string[]
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

  class CompletionItem {
    detail?: string;
    insertText?: SnippetString;
    sortText?: string;

    constructor(
      readonly label: string,
      readonly kind: number
    ) {}
  }

  class SnippetString {
    constructor(readonly value: string) {}
  }

  return {
    CompletionItem,
    CompletionItemKind: {
      Field: 4,
      Function: 2,
      Keyword: 14,
      Snippet: 15,
      Struct: 7,
      TypeParameter: 24,
      Variable: 5
    },
    Diagnostic,
    DiagnosticSeverity: { Error: 0 },
    languages: {
      registerCompletionItemProvider: (_selector: unknown, provider: typeof vscodeMockState.registeredProvider, ...triggerCharacters: string[]) => {
        vscodeMockState.registeredProvider = provider;
        vscodeMockState.triggerCharacters = triggerCharacters;
        return { dispose: vi.fn() };
      }
    },
    Position,
    Range,
    SnippetString,
    Uri: {
      parse: (uri: string) => ({
        toString: () => uri
      })
    }
  };
});

import * as vscode from "vscode";
import { buildCompletionItems, registerCompletions } from "../src/completions";
import { analyzeIntKernelDocument, createMemoryDocument } from "../src/languageService";

const sourceText = `
struct Item {
  price: i64;
  qty: i64;
}

fn total(item: Item) -> i64 {
  let subtotal: i64 = item.price;
  return subtotal;
}
`.trimStart();

describe("completions", () => {
  it("includes static keywords and document symbols", () => {
    const analysis = analyzeIntKernelDocument(createMemoryDocument(sourceText));
    const items = buildCompletionItems(analysis, new vscode.Position(7, 9));
    const labels = items.map((item) => item.label.toString());
    expect(labels).toContain("while");
    expect(labels).toContain("subtotal");
    expect(labels).toContain("total");
    expect(labels).toContain("Item");
  });

  it("suggests struct fields after member access", () => {
    const analysis = analyzeIntKernelDocument(createMemoryDocument(sourceText));
    const items = buildCompletionItems(analysis, new vscode.Position(6, 30), "item.");
    const labels = items.map((item) => item.label.toString());
    expect(labels).toContain("price");
    expect(labels).toContain("qty");
    expectMemberFieldsToSortBeforeStaticItems(items, ["price", "qty"]);
  });

  it("uses the nearest receiver when names repeat", () => {
    const repeatedReceiverSourceText = `
struct Item {
  price: i64;
}

struct Other {
  code: i64;
}

fn first(item: Item) -> i64 {
  return item.price;
}

fn second(item: Other) -> i64 {
  return item.code;
}
`.trimStart();
    const analysis = analyzeIntKernelDocument(createMemoryDocument(repeatedReceiverSourceText, "memory:///repeated-receiver.ik"));
    const items = buildCompletionItems(analysis, new vscode.Position(13, 14), "item.");
    const labels = items.map((item) => item.label.toString());

    expect(labels).toContain("code");
    expect(labels).not.toContain("price");
  });

  it("does not include locals or parameters that are not visible at the cursor", () => {
    const scopedSourceText = `
fn first(other_param: i64) -> i64 {
  let other_local: i64 = other_param;
  return other_local;
}

fn second(current_param: i64) -> i64 {
  let before_cursor: i64 = current_param;
  let after_cursor: i64 = before_cursor;
  return after_cursor;
}
`.trimStart();
    const analysis = analyzeIntKernelDocument(createMemoryDocument(scopedSourceText, "memory:///scoped-completions.ik"));
    const items = buildCompletionItems(analysis, new vscode.Position(6, 30));
    const labels = items.map((item) => item.label.toString());

    expect(labels).toContain("current_param");
    expect(labels).toContain("before_cursor");
    expect(labels).not.toContain("other_param");
    expect(labels).not.toContain("other_local");
    expect(labels).not.toContain("after_cursor");
  });

  it("does not include block-scoped locals outside the block", () => {
    const blockScopedSourceText = `
fn total(value: i64) -> i64 {
  if true {
    let branch_only: i64 = value;
  }
  return value;
}
`.trimStart();
    const analysis = analyzeIntKernelDocument(createMemoryDocument(blockScopedSourceText, "memory:///block-scoped-completions.ik"));
    const items = buildCompletionItems(analysis, new vscode.Position(4, 9));
    const labels = items.map((item) => item.label.toString());

    expect(labels).toContain("value");
    expect(labels).not.toContain("branch_only");
  });

  it("does not suggest struct fields for pointer receivers", () => {
    const pointerSourceText = `
struct Item {
  price: i64;
}

fn total(item: ptr<Item>) -> i64 {
  return 0;
}
`.trimStart();
    const analysis = analyzeIntKernelDocument(createMemoryDocument(pointerSourceText, "memory:///pointer-receiver-completions.ik"));
    const items = buildCompletionItems(analysis, new vscode.Position(5, 14), "item.");
    const labels = items.map((item) => item.label.toString());

    expect(labels).not.toContain("price");
  });

  it("registers dot-triggered provider completions for member access", () => {
    vscodeMockState.registeredProvider = undefined;
    vscodeMockState.triggerCharacters = [];
    const context = { subscriptions: [] as Array<{ dispose(): void }> };
    registerCompletions(context as vscode.ExtensionContext);
    const document = {
      ...createMemoryDocument(sourceText, "memory:///registered-completions.ik"),
      lineAt: (position: vscode.Position) => ({
        text: sourceText.split("\n")[position.line] ?? ""
      })
    };

    const provider = vscodeMockState.registeredProvider as
      | {
          provideCompletionItems(document: unknown, position: unknown): vscode.CompletionItem[];
        }
      | undefined;
    const items = provider?.provideCompletionItems(document, new vscode.Position(6, 27));
    const labels = items?.map((item) => item.label.toString()) ?? [];

    expect(context.subscriptions).toHaveLength(1);
    expect(vscodeMockState.triggerCharacters).toEqual(["."]);
    expect(labels).toContain("price");
    expect(labels).toContain("qty");
  });
});

function sortTextFor(items: vscode.CompletionItem[], label: string): string | undefined {
  return items.find((item) => item.label.toString() === label)?.sortText;
}

function expectMemberFieldsToSortBeforeStaticItems(items: vscode.CompletionItem[], fieldLabels: string[]): void {
  const fieldSortTexts = fieldLabels.map((label) => sortTextFor(items, label));
  const staticSortTexts = items
    .filter((item) => !fieldLabels.includes(item.label.toString()))
    .map((item) => item.sortText)
    .filter((sortText): sortText is string => Boolean(sortText));

  for (const fieldSortText of fieldSortTexts) {
    expect(fieldSortText).toBeDefined();
    for (const staticSortText of staticSortTexts) {
      expect(fieldSortText?.localeCompare(staticSortText)).toBeLessThan(0);
    }
  }
}
